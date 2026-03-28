use std::path::PathBuf;

use clap::Parser;
use rig::agent::AgentBuilder;
use rig::client::CompletionClient;
use rig::completion::request::Prompt;
use skeet_scorer::examples::{Example, load_examples};
use skeet_scorer::model::{ModelName, ModelProvider, ScoringModel, ScoringPrompt, save_model};
use skeet_scorer::scoring::{SEED_PROMPT, build_agent, create_client, score_image};
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(name = "train", about = "Train a scoring prompt by iterating over examples")]
struct Args {
    /// Path to expected.toml
    #[arg(long, default_value = "examples/expected.toml")]
    examples_path: PathBuf,

    /// Directory containing example images
    #[arg(long, default_value = "examples")]
    examples_dir: PathBuf,

    /// Path to write the resulting model.toml
    #[arg(long, default_value = "skeet-scorer/model.toml")]
    model_output: PathBuf,

    /// OpenAI API key
    #[arg(long, env = "BOBBY_OPENAI_API_KEY")]
    openai_api_key: String,

    /// Maximum training iterations
    #[arg(long, default_value_t = 10)]
    max_iterations: u32,

    /// Target accuracy threshold (0.0-1.0) to stop early
    #[arg(long, default_value_t = 0.8)]
    target_accuracy: f32,
}

struct ScoredExample {
    path: String,
    exemplar: bool,
    score: f32,
    correct: bool,
}

fn is_correct(exemplar: bool, score: f32) -> bool {
    if exemplar { score > 0.5 } else { score <= 0.5 }
}

async fn score_examples(
    agent: &rig::agent::Agent<rig::providers::openai::CompletionModel>,
    examples: &[Example],
    examples_dir: &std::path::Path,
) -> Vec<ScoredExample> {
    let mut results = Vec::new();

    for example in examples {
        let image_path = examples_dir.join(&example.path);
        let image = match image::open(&image_path) {
            Ok(img) => img,
            Err(e) => {
                warn!(path = %image_path.display(), error = %e, "failed to load example image, skipping");
                continue;
            }
        };

        match score_image(agent, &image).await {
            Ok(score) => {
                let score_f32: f32 = score.into();
                let correct = is_correct(example.exemplar, score_f32);
                info!(
                    path = %example.path,
                    exemplar = example.exemplar,
                    score = score_f32,
                    correct,
                    "scored example"
                );
                results.push(ScoredExample {
                    path: example.path.clone(),
                    exemplar: example.exemplar,
                    score: score_f32,
                    correct,
                });
            }
            Err(e) => {
                error!(path = %example.path, error = %e, "failed to score example");
            }
        }
    }

    results
}

fn accuracy(results: &[ScoredExample]) -> f32 {
    if results.is_empty() {
        return 0.0;
    }
    let correct = results.iter().filter(|r| r.correct).count();
    correct as f32 / results.len() as f32
}

fn format_results_for_refinement(results: &[ScoredExample]) -> String {
    let mut s = String::from("Here are the scoring results for each example:\n\n");
    for r in results {
        let status = if r.correct { "CORRECT" } else { "WRONG" };
        let expected = if r.exemplar { "high (>0.5)" } else { "low (<=0.5)" };
        s.push_str(&format!(
            "- {}: score={:.2}, expected={}, status={}\n",
            r.path, r.score, expected, status
        ));
    }
    s
}

async fn refine_prompt(
    client: &rig::providers::openai::client::CompletionsClient,
    current_prompt: &str,
    results: &[ScoredExample],
) -> Result<String, Box<dyn std::error::Error>> {
    let results_summary = format_results_for_refinement(results);
    let acc = accuracy(results);

    let refinement_request = format!(
        "You are helping improve a scoring prompt for an image classification system.\n\n\
         The current scoring prompt is:\n\
         ---\n{current_prompt}\n---\n\n\
         {results_summary}\n\
         Current accuracy: {acc:.0}%\n\n\
         The exemplar=true images are good selfies with landmarks. The exemplar=false images should get low scores.\n\n\
         Please provide an improved scoring prompt that would better distinguish between good and bad examples.\n\
         Respond with ONLY the new prompt text, nothing else. Do not include any preamble or explanation."
    );

    let refinement_model = client.completion_model("gpt-4o");
    let agent = AgentBuilder::new(refinement_model).build();
    let response = agent.prompt(refinement_request).await?;
    Ok(response)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let examples = load_examples(&args.examples_path)?;
    info!(count = examples.len(), "loaded examples");

    let client = create_client(&args.openai_api_key);

    let mut current_prompt = SEED_PROMPT.to_string();
    let mut best_prompt = current_prompt.clone();
    let mut best_accuracy = 0.0_f32;

    for iteration in 1..=args.max_iterations {
        info!(iteration, "starting training iteration");

        let agent = build_agent(&client, "gpt-4o", &current_prompt);
        let results = score_examples(&agent, &examples, &args.examples_dir).await;
        let acc = accuracy(&results);

        info!(iteration, accuracy = format!("{:.0}%", acc * 100.0), "iteration complete");

        if acc > best_accuracy {
            best_accuracy = acc;
            best_prompt = current_prompt.clone();
            info!(best_accuracy = format!("{:.0}%", best_accuracy * 100.0), "new best prompt");
        }

        if acc >= args.target_accuracy {
            info!("target accuracy reached, stopping");
            break;
        }

        if iteration < args.max_iterations {
            info!("refining prompt...");
            match refine_prompt(&client, &current_prompt, &results).await {
                Ok(new_prompt) => {
                    info!(prompt_length = new_prompt.len(), "got refined prompt");
                    current_prompt = new_prompt;
                }
                Err(e) => {
                    error!(error = %e, "failed to refine prompt, keeping current");
                }
            }
        }
    }

    let model = ScoringModel {
        model_provider: ModelProvider::openai(),
        model_name: ModelName::gpt_4o(),
        prompt: ScoringPrompt::new(best_prompt),
    };
    save_model(&args.model_output, &model)?;
    info!(
        path = %args.model_output.display(),
        accuracy = format!("{:.0}%", best_accuracy * 100.0),
        "saved model"
    );

    Ok(())
}
