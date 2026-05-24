use std::collections::HashMap;

use eval::LabelledScore;
use futures::stream::{self, StreamExt};
use image::DynamicImage;
use shared::{ImageId, Score};
use tracing::{info, warn};

use crate::loader::LabelledImage;
use crate::refining::{ResilientScore, ScoringOutcome};

#[derive(Debug, Clone, Copy)]
pub struct ScoredCall {
    pub score: Score,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub outcome: ScoringOutcome,
}

impl From<ResilientScore> for ScoredCall {
    fn from(r: ResilientScore) -> Self {
        Self {
            score: r.score,
            input_tokens: r.usage.input_tokens,
            output_tokens: r.usage.output_tokens,
            outcome: r.outcome,
        }
    }
}

/// Score every input image concurrently, keyed by `ImageId` in the returned map.
///
/// The scorer closure is expected to be infallible — typically
/// [`crate::refining::refine_image_resilient`] — so call-site errors are
/// already absorbed into `ResilientScore::FallbackAfterRetries`. This
/// function repackages each result into a `ScoredCall` keyed by image id.
pub async fn score_concurrent<F, Fut>(
    images: &[LabelledImage],
    concurrency: usize,
    scorer: F,
) -> HashMap<ImageId, ScoredCall>
where
    F: Fn(DynamicImage) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = ResilientScore> + Send,
{
    let total = images.len();
    stream::iter(images.iter())
        .map(|labelled| {
            let id = labelled.id.clone();
            let fut = scorer(labelled.image.clone());
            async move {
                let resilient = fut.await;
                (id, ScoredCall::from(resilient))
            }
        })
        .buffered(concurrency)
        .enumerate()
        .map(|(i, (id, call))| {
            if call.outcome == ScoringOutcome::FallbackAfterRetries {
                warn!(idx = i, %id, "scoring fell back after exhausted retries");
            } else if i % 10 == 0 {
                info!(idx = i, total, "scoring progress");
            }
            (id, call)
        })
        .collect()
        .await
}

/// Count of fallback scores in the result map. Used to surface a "how many
/// scores were sentinels" signal in logs and printed summaries.
pub fn fallback_count(scored: &HashMap<ImageId, ScoredCall>) -> usize {
    scored
        .values()
        .filter(|c| c.outcome == ScoringOutcome::FallbackAfterRetries)
        .count()
}

pub fn labelled_scores(
    images: &[LabelledImage],
    scored: &HashMap<ImageId, ScoredCall>,
) -> Vec<LabelledScore> {
    images
        .iter()
        .map(|img| {
            let call = scored
                .get(&img.id)
                .expect("score_concurrent produced a result for every input image");
            LabelledScore {
                score: call.score,
                is_positive: img.is_positive(),
            }
        })
        .collect()
}

pub fn token_totals(scored: &HashMap<ImageId, ScoredCall>) -> (u64, u64) {
    scored.values().fold((0, 0), |(i, o), c| {
        (i + c.input_tokens, o + c.output_tokens)
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rig::completion::Usage;
    use shared::Band;
    use test_support::{marker_image, marker_of, score_for};
    use tokio::sync::Mutex;

    use super::*;

    fn labelled(marker: u32, band: Band) -> LabelledImage {
        let img = marker_image(marker);
        let id = ImageId::from_image(&img);
        LabelledImage {
            id,
            image: img,
            band,
        }
    }

    fn usage(input_tokens: u64, output_tokens: u64) -> Usage {
        let mut u = Usage::new();
        u.input_tokens = input_tokens;
        u.output_tokens = output_tokens;
        u.total_tokens = input_tokens + output_tokens;
        u
    }

    fn scored_ok(score: Score, usage: Usage) -> ResilientScore {
        ResilientScore {
            score,
            usage,
            duration: Duration::ZERO,
            outcome: ScoringOutcome::Scored,
        }
    }

    fn fallback() -> ResilientScore {
        ResilientScore {
            score: Score::new(0.0).expect("valid"),
            usage: Usage::new(),
            duration: Duration::ZERO,
            outcome: ScoringOutcome::FallbackAfterRetries,
        }
    }

    #[tokio::test]
    async fn each_id_receives_its_own_score_under_reversed_completion_order() {
        let images: Vec<LabelledImage> =
            (0..4_u32).map(|m| labelled(m, Band::HighQuality)).collect();

        let scored = score_concurrent(&images, 4, |img| async move {
            let m = marker_of(&img);
            // Higher markers complete first so completion order != submission order.
            let delay_ms = u64::from(4 - m) * 25;
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            scored_ok(score_for(m), usage(10 + u64::from(m), u64::from(m)))
        })
        .await;

        for img in &images {
            let m = marker_of(&img.image);
            let call = scored.get(&img.id).expect("score for id");
            assert_eq!(call.score, score_for(m));
            assert_eq!(call.input_tokens, 10 + u64::from(m));
            assert_eq!(call.output_tokens, u64::from(m));
            assert_eq!(call.outcome, ScoringOutcome::Scored);
        }
        assert_eq!(scored.len(), images.len());
    }

    #[tokio::test]
    async fn fallback_outcomes_pass_through_to_scored_call() {
        let images = vec![labelled(0, Band::Low), labelled(1, Band::HighQuality)];

        let scored = score_concurrent(&images, 2, |_img| async move { fallback() }).await;

        assert_eq!(scored.len(), images.len());
        for img in &images {
            let call = scored.get(&img.id).expect("score for id");
            assert_eq!(call.outcome, ScoringOutcome::FallbackAfterRetries);
            assert_eq!(f64::from(call.score), 0.0);
        }
        assert_eq!(fallback_count(&scored), images.len());
    }

    #[tokio::test]
    async fn calls_scorer_once_per_image() {
        let images: Vec<LabelledImage> = (0..5_u32).map(|m| labelled(m, Band::Low)).collect();
        let calls: std::sync::Arc<Mutex<Vec<u32>>> = std::sync::Arc::new(Mutex::new(Vec::new()));

        let calls_for_scorer = calls.clone();
        let _ = score_concurrent(&images, 2, move |img| {
            let calls = calls_for_scorer.clone();
            async move {
                calls.lock().await.push(marker_of(&img));
                scored_ok(score_for(0), usage(1, 1))
            }
        })
        .await;

        let mut seen = calls.lock().await.clone();
        seen.sort_unstable();
        assert_eq!(seen, vec![0, 1, 2, 3, 4]);
    }

    fn scored_call(input_tokens: u64, output_tokens: u64) -> ScoredCall {
        ScoredCall {
            score: Score::new(0.5).expect("valid"),
            input_tokens,
            output_tokens,
            outcome: ScoringOutcome::Scored,
        }
    }

    #[test]
    fn token_totals_sums_inputs_and_outputs_independently() {
        let mut scored: HashMap<ImageId, ScoredCall> = HashMap::new();
        scored.insert(ImageId::from_image(&marker_image(0)), scored_call(10, 1));
        scored.insert(ImageId::from_image(&marker_image(1)), scored_call(20, 2));
        scored.insert(ImageId::from_image(&marker_image(2)), scored_call(30, 3));

        assert_eq!(token_totals(&scored), (60, 6));
    }

    #[test]
    fn token_totals_of_empty_is_zero() {
        let scored: HashMap<ImageId, ScoredCall> = HashMap::new();
        assert_eq!(token_totals(&scored), (0, 0));
    }

    #[test]
    fn labelled_scores_pairs_by_id_not_position() {
        // Build images in marker order 0, 1, 2; insert scored entries in
        // reverse order so HashMap iteration is unlikely to mirror input order.
        let images: Vec<LabelledImage> = vec![
            labelled(0, Band::HighQuality),
            labelled(1, Band::Low),
            labelled(2, Band::HighQuality),
        ];
        let mut scored: HashMap<ImageId, ScoredCall> = HashMap::new();
        scored.insert(
            images[2].id.clone(),
            ScoredCall {
                score: Score::new(0.30).expect("valid"),
                input_tokens: 0,
                output_tokens: 0,
                outcome: ScoringOutcome::Scored,
            },
        );
        scored.insert(
            images[1].id.clone(),
            ScoredCall {
                score: Score::new(0.10).expect("valid"),
                input_tokens: 0,
                output_tokens: 0,
                outcome: ScoringOutcome::Scored,
            },
        );
        scored.insert(
            images[0].id.clone(),
            ScoredCall {
                score: Score::new(0.00).expect("valid"),
                input_tokens: 0,
                output_tokens: 0,
                outcome: ScoringOutcome::Scored,
            },
        );

        let labelled = labelled_scores(&images, &scored);

        assert_eq!(labelled.len(), 3);
        assert_eq!(f32::from(labelled[0].score), 0.00);
        assert!(labelled[0].is_positive); // HighQuality
        assert_eq!(f32::from(labelled[1].score), 0.10);
        assert!(!labelled[1].is_positive); // Low
        assert_eq!(f32::from(labelled[2].score), 0.30);
        assert!(labelled[2].is_positive); // HighQuality
    }

    #[test]
    fn labelled_scores_of_no_images_is_empty() {
        let images: Vec<LabelledImage> = Vec::new();
        let scored: HashMap<ImageId, ScoredCall> = HashMap::new();
        assert!(labelled_scores(&images, &scored).is_empty());
    }
}
