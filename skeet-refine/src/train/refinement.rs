use std::collections::HashMap;

use eval::{Threshold, confusion_at};
use rig::agent::AgentBuilder;
use rig::client::CompletionClient;
use rig::completion::request::Prompt;
use rig::providers::openai::client::CompletionsClient;
use shared::ImageId;

use crate::loader::LabelledImage;
use crate::train::gate::training_loop_threshold;
use crate::train::scoring::{ScoredCall, labelled_scores};

#[derive(Debug, thiserror::Error)]
#[error("no scoring result for image id {0}")]
pub struct MissingScore(pub ImageId);

/// Render a per-image scoring summary for the prompt-refinement LLM. A row is
/// labelled `CORRECT` when the ground-truth label and the prediction at
/// `threshold` agree; otherwise `WRONG`.
fn format_results_for_refinement(
    images: &[LabelledImage],
    scored: &HashMap<ImageId, ScoredCall>,
    threshold: Threshold,
) -> Result<String, MissingScore> {
    let mut s = String::from("Here are the scoring results for each example:\n\n");
    for img in images.iter() {
        let call = scored
            .get(&img.id)
            .ok_or_else(|| MissingScore(img.id.clone()))?;
        let is_pos = img.is_positive();
        let score: f32 = call.score.into();
        let predicted_pos = Threshold::from(call.score) >= threshold;
        let status = if is_pos == predicted_pos {
            "CORRECT"
        } else {
            "WRONG"
        };
        let expected = if is_pos {
            format!("high (>={threshold})")
        } else {
            format!("low (<{threshold})")
        };
        s.push_str(&format!(
            "- {}: score={:.2}, expected={}, status={}\n",
            img.id, score, expected, status
        ));
    }
    Ok(s)
}

pub async fn refine_prompt(
    client: &CompletionsClient,
    model_name: &str,
    current_prompt: &str,
    images: &[LabelledImage],
    scored: &HashMap<ImageId, ScoredCall>,
) -> Result<String, Box<dyn std::error::Error>> {
    let threshold = training_loop_threshold();
    let labelled = labelled_scores(images, scored);
    let matrix = confusion_at(&labelled, threshold);
    let f1_pct = matrix
        .f1()
        .map(|v| format!("{:.0}%", f64::from(v) * 100.0))
        .unwrap_or_else(|| "undefined".to_string());
    let results_summary = format_results_for_refinement(images, scored, threshold)?;

    let refinement_request = format!(
        "You are helping improve a scoring prompt for an image classification system.\n\n\
         The current scoring prompt is:\n\
         ---\n{current_prompt}\n---\n\n\
         {results_summary}\n\
         Current train F1: {f1_pct}\n\n\
         The expected=high images are good selfies with landmarks. The expected=low images should get low scores.\n\n\
         Please provide an improved scoring prompt that would better distinguish between good and bad examples.\n\
         Respond with ONLY the new prompt text, nothing else. Do not include any preamble or explanation."
    );

    let refinement_model = client.completion_model(model_name);
    let agent = AgentBuilder::new(refinement_model).temperature(0.0).build();
    let response = agent.prompt(refinement_request).await?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use shared::{Band, ImageId, Score};
    use test_support::marker_image;

    use super::*;
    use crate::loader::LabelledImage;
    use crate::refining::ScoringOutcome;

    fn labelled(marker: u32, band: Band) -> LabelledImage {
        let img = marker_image(marker);
        let id = ImageId::from_image(&img);
        LabelledImage {
            id,
            image: img,
            band,
        }
    }

    fn scored_call(score: f32) -> ScoredCall {
        ScoredCall {
            score: Score::new(score).expect("valid"),
            input_tokens: 0,
            output_tokens: 0,
            outcome: ScoringOutcome::Scored,
        }
    }

    fn scored_for(image: &LabelledImage, score: f32) -> (ImageId, ScoredCall) {
        (image.id.clone(), scored_call(score))
    }

    fn threshold(value: f64) -> Threshold {
        Threshold::new(value).expect("in [0.0, 1.0]")
    }

    #[test]
    fn correct_when_predicted_class_matches_label() {
        let pos = labelled(0, Band::HighQuality);
        let neg = labelled(1, Band::Low);
        let scored: HashMap<ImageId, ScoredCall> =
            [scored_for(&pos, 0.9), scored_for(&neg, 0.1)].into();

        let s = format_results_for_refinement(&[pos, neg], &scored, threshold(0.5))
            .expect("scored covers every image");
        assert!(s.contains("status=CORRECT"));
        assert!(!s.contains("status=WRONG"));
    }

    #[test]
    fn wrong_when_positive_label_scored_below_threshold() {
        let pos = labelled(0, Band::HighQuality);
        let scored: HashMap<ImageId, ScoredCall> = [scored_for(&pos, 0.3)].into();

        let s = format_results_for_refinement(&[pos], &scored, threshold(0.5))
            .expect("scored covers every image");
        assert!(s.contains("status=WRONG"));
    }

    #[test]
    fn wrong_when_negative_label_scored_above_threshold() {
        let neg = labelled(0, Band::Low);
        let scored: HashMap<ImageId, ScoredCall> = [scored_for(&neg, 0.7)].into();

        let s = format_results_for_refinement(&[neg], &scored, threshold(0.5))
            .expect("scored covers every image");
        assert!(s.contains("status=WRONG"));
    }

    #[test]
    fn at_threshold_counts_as_positive_prediction() {
        let pos = labelled(0, Band::HighQuality);
        let scored: HashMap<ImageId, ScoredCall> = [scored_for(&pos, 0.5)].into();

        let s = format_results_for_refinement(&[pos], &scored, threshold(0.5))
            .expect("scored covers every image");
        assert!(s.contains("status=CORRECT"));
    }

    #[test]
    fn expected_label_text_reflects_threshold() {
        let pos = labelled(0, Band::HighQuality);
        let neg = labelled(1, Band::Low);
        let scored: HashMap<ImageId, ScoredCall> =
            [scored_for(&pos, 0.5), scored_for(&neg, 0.5)].into();

        let s = format_results_for_refinement(&[pos, neg], &scored, threshold(0.6))
            .expect("scored covers every image");
        assert!(s.contains("expected=high (>=0.600)"));
        assert!(s.contains("expected=low (<0.600)"));
    }

    #[test]
    fn includes_one_row_per_image_in_input_order() {
        let images: Vec<LabelledImage> =
            (0..3_u32).map(|m| labelled(m, Band::HighQuality)).collect();
        let scored: HashMap<ImageId, ScoredCall> = images
            .iter()
            .enumerate()
            .map(|(i, img)| scored_for(img, 0.1 * (i as f32 + 1.0)))
            .collect();

        let s = format_results_for_refinement(&images, &scored, threshold(0.5))
            .expect("scored covers every image");

        let row_count = s.lines().filter(|l| l.starts_with('-')).count();
        assert_eq!(row_count, 3);

        // Position of each id in the formatted output matches input order.
        let p0 = s.find(&images[0].id.to_string()).expect("img 0 present");
        let p1 = s.find(&images[1].id.to_string()).expect("img 1 present");
        let p2 = s.find(&images[2].id.to_string()).expect("img 2 present");
        assert!(p0 < p1 && p1 < p2);
    }

    #[test]
    fn empty_input_produces_header_only() {
        let images: Vec<LabelledImage> = Vec::new();
        let scored: HashMap<ImageId, ScoredCall> = HashMap::new();
        let s = format_results_for_refinement(&images, &scored, threshold(0.5))
            .expect("no images to look up");
        assert_eq!(s, "Here are the scoring results for each example:\n\n");
    }

    #[test]
    fn errors_when_score_missing_for_an_image() {
        let img = labelled(0, Band::HighQuality);
        let scored: HashMap<ImageId, ScoredCall> = HashMap::new();

        let err = format_results_for_refinement(&[img], &scored, threshold(0.5))
            .expect_err("scored is empty");
        let MissingScore(missing_id) = err;
        assert_eq!(missing_id, ImageId::from_image(&marker_image(0)));
    }
}
