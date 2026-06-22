use std::path::PathBuf;

use clap::{Parser, Subcommand};
use shared::ModelVersion;
use skeet_refine::model::{Label, RefineModels};

#[derive(Parser)]
#[command(
    name = "promote",
    about = "Inspect and repoint the production label in refine.toml. Promotion is label-only: it moves which registered model is `production`, with no data migration. The k8s image flip is a separate manual step (see docs/versioning.md)."
)]
struct Args {
    /// Path to the refine model registry
    #[arg(long, default_value = "config/refine.toml")]
    model_path: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show every label and the registered models it could point at.
    Show,
    /// Repoint the `production` label at an already-registered model version.
    Set {
        /// The target model_version (must already be registered in refine.toml).
        version: String,
    },
}

fn show(models: &RefineModels) {
    println!("labels:");
    let mut labels: Vec<_> = models.labels().collect();
    labels.sort_by_key(|(label, _)| label.to_string());
    for (label, version) in labels {
        println!("  {label} -> {version}");
    }

    println!("registered models:");
    let mut versions: Vec<_> = models.versions().collect();
    versions.sort_by_key(ToString::to_string);
    for version in versions {
        let name = models
            .get(version)
            .map_or_else(|| "?".to_string(), |m| m.model_name.to_string());
        println!("  {version} ({name})");
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let mut models = RefineModels::load(&args.model_path)?;

    match args.command {
        Command::Show => show(&models),
        Command::Set { version } => {
            let version = ModelVersion::from(version.as_str());
            models.set_label(Label::production(), version.clone())?;
            models.save(&args.model_path)?;
            println!("production -> {version}");
        }
    }
    Ok(())
}
