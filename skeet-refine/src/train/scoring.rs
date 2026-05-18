use std::collections::HashMap;

use eval::LabelledScore;
use futures::stream::{self, StreamExt};
use shared::Score;
use skeet_store::ImageId;
use tracing::{error, info};

use crate::loader::LabelledImage;
use crate::refining::{RefineAgent, RefineError, refine_image};

#[derive(Debug, Clone, Copy)]
pub struct ScoredCall {
    pub score: Score,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Concurrently score every input image. The returned map is keyed by
/// `ImageId` so callers must look up by id rather than relying on the order
/// of futures completion.
pub async fn score_concurrent(
    agent: &RefineAgent,
    images: &[LabelledImage],
    concurrency: usize,
) -> Result<HashMap<ImageId, ScoredCall>, RefineError> {
    let total = images.len();
    stream::iter(images.iter())
        .map(|labelled| {
            let id = labelled.id.clone();
            async move {
                refine_image(agent, &labelled.image).await.map(|(score, usage, _d)| {
                    (
                        id,
                        ScoredCall {
                            score,
                            input_tokens: usage.input_tokens,
                            output_tokens: usage.output_tokens,
                        },
                    )
                })
            }
        })
        .buffered(concurrency)
        .enumerate()
        .map(|(i, r)| {
            if let Err(e) = &r {
                error!(idx = i, error = %e, "scoring failed");
            } else if i % 10 == 0 {
                info!(idx = i, total, "scoring progress");
            }
            r
        })
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect()
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
