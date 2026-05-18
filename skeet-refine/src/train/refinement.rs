use std::collections::HashMap;

use eval::confusion_at;
use rig::agent::AgentBuilder;
use rig::client::CompletionClient;
use rig::completion::request::Prompt;
use rig::providers::openai::client::CompletionsClient;
use skeet_store::ImageId;

use crate::loader::LabelledImage;
use crate::train::gate::training_loop_threshold;
use crate::train::scoring::{ScoredCall, labelled_scores};

fn format_results_for_refinement(
    images: &[LabelledImage],
    scored: &HashMap<ImageId, ScoredCall>,
) -> String {
    let mut s = String::from("Here are the scoring results for each example:\n\n");
    for img in images.iter() {
        let call = scored
            .get(&img.id)
            .expect("score_concurrent produced a result for every input image");
        let is_pos = img.is_positive();
        let score: f32 = call.score.into();
        let predicted_pos = score > 0.5;
        let status = if is_pos == predicted_pos { "CORRECT" } else { "WRONG" };
        let expected = if is_pos { "high (>0.5)" } else { "low (<=0.5)" };
        s.push_str(&format!(
            "- {}: score={:.2}, expected={}, status={}\n",
            img.id, score, expected, status
        ));
    }
    s
}

pub async fn refine_prompt(
    client: &CompletionsClient,
    model_name: &str,
    current_prompt: &str,
    images: &[LabelledImage],
    scored: &HashMap<ImageId, ScoredCall>,
) -> Result<String, Box<dyn std::error::Error>> {
    let labelled = labelled_scores(images, scored);
    let matrix = confusion_at(&labelled, training_loop_threshold());
    let f1_pct = matrix
        .f1()
        .map(|v| format!("{:.0}%", f64::from(v) * 100.0))
        .unwrap_or_else(|| "undefined".to_string());
    let results_summary = format_results_for_refinement(images, scored);

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
