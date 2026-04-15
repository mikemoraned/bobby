use std::io::Cursor;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use image::DynamicImage;
use rig::agent::{Agent, AgentBuilder};
use rig::client::CompletionClient;
use rig::completion::message::{ImageDetail, ImageMediaType, Message, UserContent};
use rig::completion::request::Prompt;
use rig::one_or_many::OneOrMany;
use rig::providers::openai;
use shared::Score;
use tracing::{info, instrument};

#[derive(Debug, thiserror::Error)]
pub enum RefineError {
    #[error("image encoding error: {0}")]
    ImageEncoding(#[from] image::ImageError),

    #[error("LLM completion error: {0}")]
    Completion(String),

    #[error("failed to parse score from LLM response: {0}")]
    ParseScore(String),
}

fn encode_image_base64(image: &DynamicImage) -> Result<String, RefineError> {
    let mut buf = Cursor::new(Vec::new());
    image.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(BASE64.encode(buf.into_inner()))
}

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

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str)
        && let Some(score) = v.get("score").and_then(|s| s.as_f64())
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
) -> Result<Score, RefineError> {
    let msg = build_image_message(image)?;

    let response = agent
        .prompt(msg)
        .await
        .map_err(|e| RefineError::Completion(e.to_string()))?;

    info!(response = %response, "LLM response");
    parse_score(&response)
}

pub fn build_agent(
    client: &openai::client::CompletionsClient,
    model_name: &str,
    refine_prompt: &str,
) -> RefineAgent {
    let model = client.completion_model(model_name);
    AgentBuilder::new(model)
        .preamble(refine_prompt)
        .build()
}

pub fn create_client(api_key: &str) -> openai::client::CompletionsClient {
    openai::client::CompletionsClient::new(api_key)
        .expect("failed to create OpenAI client")
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
    fn parse_score_rejects_garbage() {
        assert!(parse_score("hello world").is_err());
    }
}
