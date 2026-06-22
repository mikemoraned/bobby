use std::io::Cursor;
use std::time::{Duration, Instant};

use backon::{ExponentialBuilder, Retryable};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use image::DynamicImage;
use rig::agent::{Agent, AgentBuilder};
use rig::client::CompletionClient;
use rig::completion::message::{
    AssistantContent, ImageDetail, ImageMediaType, Message, UserContent,
};
use rig::completion::{Completion, Usage};
use rig::one_or_many::OneOrMany;
use rig::providers::openai;
use shared::Score;
use tracing::{info, instrument, warn};

#[derive(Debug, thiserror::Error)]
pub enum RefineError {
    #[error("image encoding error: {0}")]
    ImageEncoding(#[from] image::ImageError),

    #[error("LLM completion error: {0}")]
    Completion(String),

    #[error("failed to parse score from LLM response: {0}")]
    ParseScore(String),
}

impl RefineError {
    pub const fn as_label(&self) -> &'static str {
        match self {
            Self::ImageEncoding(_) => "ImageEncoding",
            Self::Completion(_) => "Completion",
            Self::ParseScore(_) => "ParseScore",
        }
    }
}

fn encode_image_base64(image: &DynamicImage) -> Result<String, RefineError> {
    let mut buf = Cursor::new(Vec::new());
    image.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(BASE64.encode(buf.into_inner()))
}

#[allow(clippy::expect_used)] // the content vec is a non-empty literal
fn build_image_message(image: &DynamicImage) -> Result<Message, RefineError> {
    let b64 = encode_image_base64(image)?;
    Ok(Message::User {
        content: OneOrMany::many(vec![
            UserContent::text(
                "Score this image. Respond with ONLY a JSON object: {\"score\": 0.XX}",
            ),
            UserContent::image_base64(b64, Some(ImageMediaType::PNG), Some(ImageDetail::Auto)),
        ])
        .expect("non-empty content"),
    })
}

fn extract_json(text: &str) -> &str {
    let start = text.find('{').unwrap_or(0);
    let end = text.rfind('}').map_or(text.len(), |e| e + 1);
    &text[start..end]
}

fn parse_score(response: &str) -> Result<Score, RefineError> {
    let trimmed = response.trim();
    let json_str = extract_json(trimmed);

    // Accept either the requested `{"score": X}` object or a bare numeric
    // response `X` — some models drift to returning the number alone.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str)
        && let Some(score) = v
            .get("score")
            .and_then(|s| s.as_f64())
            .or_else(|| v.as_f64())
    {
        return Score::new(score as f32).map_err(|e| RefineError::ParseScore(e.to_string()));
    }

    Err(RefineError::ParseScore(format!(
        "could not extract score from: {trimmed}"
    )))
}

pub type RefineAgent = Agent<openai::completion::CompletionModel>;

#[instrument(skip(agent, image))]
pub async fn refine_image(
    agent: &RefineAgent,
    image: &DynamicImage,
) -> Result<(Score, Usage, Duration), RefineError> {
    let msg = build_image_message(image)?;

    let start = Instant::now();
    let response = agent
        .completion(msg, vec![])
        .await
        .map_err(|e| RefineError::Completion(e.to_string()))?
        .send()
        .await
        .map_err(|e| RefineError::Completion(e.to_string()))?;
    let duration = start.elapsed();

    let text = response
        .choice
        .iter()
        .find_map(|c| {
            if let AssistantContent::Text(t) = c {
                Some(t.text.clone())
            } else {
                None
            }
        })
        .ok_or_else(|| RefineError::Completion("no text content in response".to_string()))?;

    info!(response = %text, "LLM response");
    let score = parse_score(&text)?;
    Ok((score, response.usage, duration))
}

/// Outcome of a resilient scoring call.
///
/// Distinguishes a real LLM score from a zero-score substitution applied
/// after all retries were exhausted. The marker keeps the substitution honest:
/// callers can count fallbacks and weigh them against the dataset rather than
/// silently treating sentinels as real scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoringOutcome {
    Scored,
    FallbackAfterRetries,
}

/// Result of `refine_image_resilient`: either a successful score (with usage)
/// or a fallback after exhausted retries.
#[derive(Debug, Clone, Copy)]
pub struct ResilientScore {
    pub score: Score,
    pub usage: Usage,
    pub duration: Duration,
    pub outcome: ScoringOutcome,
}

/// Only Completion errors are treated as transient and worth retrying. Image
/// encoding failures will recur deterministically, and a malformed response
/// from a deterministic prompt is more likely a prompt problem than a flake.
const fn is_transient(e: &RefineError) -> bool {
    matches!(e, RefineError::Completion(_))
}

fn default_retry_policy() -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(500))
        .with_factor(2.0)
        .with_max_times(3)
}

async fn retry_refine_call<F, Fut>(
    backoff: ExponentialBuilder,
    call: F,
) -> Result<(Score, Usage, Duration), RefineError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(Score, Usage, Duration), RefineError>>,
{
    call.retry(backoff)
        .when(is_transient)
        .notify(|e, dur| {
            warn!(
                error = %e,
                retry_in_ms = dur.as_millis() as u64,
                "refine_image transient failure; will retry",
            );
        })
        .await
}

fn fallback_score(duration: Duration) -> ResilientScore {
    ResilientScore {
        score: Score::zero(),
        usage: Usage::new(),
        duration,
        outcome: ScoringOutcome::FallbackAfterRetries,
    }
}

/// `refine_image` with bounded exponential-backoff retries on transient errors.
///
/// Retries `Completion` errors up to three times with delays of ~0.5s, ~1.0s,
/// ~2.0s. `ImageEncoding` and `ParseScore` are not retried. When the call
/// ultimately fails, returns a fallback `ResilientScore` with `score = 0.0`
/// and `outcome = FallbackAfterRetries` — the marker keeps the sentinel
/// substitution honest at the call site.
///
/// `duration` measures total wall time including any backoff sleeps and
/// failed attempts — the operation-level latency, not the last attempt's.
#[instrument(skip(agent, image))]
pub async fn refine_image_resilient(agent: &RefineAgent, image: &DynamicImage) -> ResilientScore {
    let start = Instant::now();
    let result = retry_refine_call(default_retry_policy(), || refine_image(agent, image)).await;
    let duration = start.elapsed();
    match result {
        Ok((score, usage, _attempt_duration)) => ResilientScore {
            score,
            usage,
            duration,
            outcome: ScoringOutcome::Scored,
        },
        Err(e) => {
            warn!(error = %e, "refine_image exhausted retries; substituting fallback score=0.0");
            fallback_score(duration)
        }
    }
}

/// Temperature pinned for reproducible scoring on every model that accepts a
/// custom value.
const REPRODUCIBLE_TEMPERATURE: f64 = 0.0;

/// The temperature to request for `model`, or `None` to omit the field.
///
/// Scoring is pinned to a fixed low temperature for reproducibility. OpenAI's
/// gpt-5 reasoning family rejects any non-default temperature with a 400 and
/// must have the field omitted; the non-reasoning `gpt-5-chat` variant accepts
/// it, as do the gpt-4o / gpt-4.1 families. Other reasoning families (o-series)
/// share the gpt-5 constraint and would need adding here before use.
pub fn temperature_for(model: &str) -> Option<f64> {
    if rejects_custom_temperature(model) {
        None
    } else {
        Some(REPRODUCIBLE_TEMPERATURE)
    }
}

fn rejects_custom_temperature(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.starts_with("gpt-5") && !m.starts_with("gpt-5-chat")
}

pub fn build_agent(
    client: &openai::client::CompletionsClient,
    model_name: &str,
    refine_prompt: &str,
) -> RefineAgent {
    let model = client.completion_model(model_name);
    let mut builder = AgentBuilder::new(model).preamble(refine_prompt);
    if let Some(t) = temperature_for(model_name) {
        builder = builder.temperature(t);
    }
    builder.build()
}

// `CompletionsClient::new` only fails if the API key can't form a valid auth header —
// a startup configuration error, surfaced immediately when the binary builds its client.
#[allow(clippy::expect_used)]
pub fn create_client(api_key: &str) -> openai::client::CompletionsClient {
    openai::client::CompletionsClient::new(api_key).expect("failed to create OpenAI client")
}

pub const SEED_PROMPT: &str = r#"You are an image scoring assistant for a project that finds selfies taken by people with recognizable physical landmarks (famous buildings, monuments, places like the Eiffel Tower, Statue of Liberty, Big Ben, etc.).

Score the image between 0.0 (worst) and 1.0 (best) based on how well it matches these criteria:
- Contains exactly one person taking a selfie (face visible, looking at camera)
- A recognizable physical landmark is clearly visible in the background
- Good composition: the person and landmark are both clearly visible
- The image is a genuine selfie (not a professional photo, not a screenshot, not a meme)

A score of 1.0 means a perfect selfie with a clearly recognizable landmark.
A score of 0.0 means the image has nothing to do with selfies or landmarks."#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_score_from_json() {
        let score: f32 = parse_score(r#"{"score": 0.85}"#).expect("parse").into();
        assert!((score - 0.85).abs() < 0.001);
    }

    #[test]
    fn parse_score_from_markdown_block() {
        let response = "```json\n{\"score\": 0.42}\n```";
        let score: f32 = parse_score(response).expect("parse").into();
        assert!((score - 0.42).abs() < 0.001);
    }

    #[test]
    fn parse_score_rejects_out_of_range() {
        assert!(parse_score(r#"{"score": 1.5}"#).is_err());
        assert!(parse_score(r#"{"score": -0.1}"#).is_err());
    }

    #[test]
    fn parse_score_from_bare_number() {
        // Some models (e.g. gpt-5-mini) return the score alone, not wrapped.
        let score: f32 = parse_score("0.05").expect("parse").into();
        assert!((score - 0.05).abs() < 0.001);
    }

    #[test]
    fn parse_score_from_bare_number_with_whitespace() {
        let score: f32 = parse_score("  0.42\n").expect("parse").into();
        assert!((score - 0.42).abs() < 0.001);
    }

    #[test]
    fn parse_score_from_bare_zero() {
        let score: f32 = parse_score("0").expect("parse").into();
        assert_eq!(f32::from(score), 0.0);
    }

    #[test]
    fn parse_score_rejects_bare_number_out_of_range() {
        assert!(parse_score("1.5").is_err());
    }

    #[test]
    fn parse_score_rejects_garbage() {
        assert!(parse_score("hello world").is_err());
    }

    #[test]
    fn temperature_pinned_to_zero_for_chat_models() {
        assert_eq!(temperature_for("gpt-4o"), Some(0.0));
        assert_eq!(temperature_for("gpt-4o-mini"), Some(0.0));
        assert_eq!(temperature_for("gpt-4.1-mini"), Some(0.0));
        assert_eq!(temperature_for("gpt-4.1-nano"), Some(0.0));
    }

    #[test]
    fn temperature_omitted_for_gpt5_reasoning_models() {
        assert_eq!(temperature_for("gpt-5"), None);
        assert_eq!(temperature_for("gpt-5-mini"), None);
        assert_eq!(temperature_for("gpt-5-nano"), None);
    }

    #[test]
    fn temperature_pinned_for_gpt5_chat_variant() {
        // The non-reasoning chat variant accepts a custom temperature.
        assert_eq!(temperature_for("gpt-5-chat-latest"), Some(0.0));
    }

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Backoff policy with negligible delay, so retry-exhaustion tests don't
    /// have to wait the production 3.5s.
    fn fast_retry_policy() -> ExponentialBuilder {
        ExponentialBuilder::default()
            .with_min_delay(Duration::from_millis(1))
            .with_factor(2.0)
            .with_max_times(3)
    }

    fn ok_result() -> Result<(Score, Usage, Duration), RefineError> {
        Ok((
            Score::new(0.5).expect("valid"),
            Usage::new(),
            Duration::ZERO,
        ))
    }

    #[test]
    fn completion_errors_are_transient() {
        assert!(is_transient(&RefineError::Completion("net down".into())));
    }

    #[test]
    fn parse_and_encoding_errors_are_not_transient() {
        assert!(!is_transient(&RefineError::ParseScore("garbage".into())));
        let img_err = image::ImageError::Limits(image::error::LimitError::from_kind(
            image::error::LimitErrorKind::DimensionError,
        ));
        assert!(!is_transient(&RefineError::ImageEncoding(img_err)));
    }

    #[tokio::test]
    async fn transient_error_then_success_returns_ok_within_retries() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_ref = calls.clone();
        let result = retry_refine_call(fast_retry_policy(), move || {
            let n = calls_ref.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 2 {
                    Err(RefineError::Completion("transient".into()))
                } else {
                    ok_result()
                }
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn exhausted_completion_retries_propagate_error() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_ref = calls.clone();
        let result = retry_refine_call(fast_retry_policy(), move || {
            calls_ref.fetch_add(1, Ordering::SeqCst);
            async move { Err(RefineError::Completion("always".into())) }
        })
        .await;

        assert!(matches!(result, Err(RefineError::Completion(_))));
        // 1 initial attempt + 3 retries = 4 calls
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn non_transient_error_is_not_retried() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_ref = calls.clone();
        let result = retry_refine_call(fast_retry_policy(), move || {
            calls_ref.fetch_add(1, Ordering::SeqCst);
            async move { Err(RefineError::ParseScore("nope".into())) }
        })
        .await;

        assert!(matches!(result, Err(RefineError::ParseScore(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn fallback_score_uses_zero_with_marker() {
        let f = fallback_score(Duration::from_millis(42));
        assert_eq!(f64::from(f.score), 0.0);
        assert_eq!(f.usage.input_tokens, 0);
        assert_eq!(f.usage.output_tokens, 0);
        assert_eq!(f.outcome, ScoringOutcome::FallbackAfterRetries);
        assert_eq!(f.duration, Duration::from_millis(42));
    }
}
