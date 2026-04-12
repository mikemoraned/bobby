#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use cot::project::Bootstrapper;
use skeet_inspect::project::InspectProject;
use skeet_store::StoreArgs;
use skeet_web_shared::StoreLayer;
use tracing::info;

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Enable tokio-console on this port
    #[arg(long)]
    tokio_console_port: Option<u16>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let console = args.tokio_console_port.map_or(
        shared::tracing::TokioConsoleSupport::Disabled,
        |port| shared::tracing::TokioConsoleSupport::Enabled { port },
    );
    let _guard = shared::tracing::init_with_file_and_stderr("skeet_inspect=info,shared=info,skeet_store=info", "inspect.log", console);
    let store = args
        .store
        .open_store()
        .await
        .expect("failed to open store at startup");

    info!("starting skeet-inspect server on 127.0.0.1:8000");

    let project = InspectProject {
        store_layer: StoreLayer::new(store),
    };
    let bootstrapper = Bootstrapper::new(project)
        .with_config_name("dev")?
        .boot()
        .await?;
    cot::run(bootstrapper, "127.0.0.1:8000").await?;
    Ok(())
}

