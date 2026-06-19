#![warn(clippy::all, clippy::nursery)]

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use clap::Parser;
use cot::project::Bootstrapper;
use shared::{Appraiser, RefineModels};
use skeet_appraise::auth_config::OAuthConfig;
use skeet_appraise::available_feeds::PublishedListCatalogReader;
use skeet_appraise::project::AppraiseProject;
use skeet_appraise::{
    AppraiserLayer, ModelsLayer, OAuthConfigLayer, PublishedFeedLayer, StartedAtLayer, StoreLayer,
};
use skeet_store::StoreArgs;
use tracing::info;

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Path to the refine model registry (refine.toml) — the display badges
    /// resolve each score → band via the producing model's threshold.
    #[arg(long, default_value = "config/refine.toml")]
    model_path: PathBuf,

    /// Address to bind the server to
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: String,

    /// Redis URL for the publish server (env: BOBBY_REDIS_PUBLISH_URL) — the
    /// home page's source of truth for what's in the feed, and where the feed
    /// catalog (the list of selectable feeds) is discovered from.
    #[arg(long, env = "BOBBY_REDIS_PUBLISH_URL")]
    redis_publish_url: String,

    /// Enable local admin mode (uses Appraiser::LocalAdmin for appraisals)
    #[arg(long, default_value_t = false)]
    local_admin: bool,

    /// GitHub OAuth client ID (env: BOBBY_GITHUB_CLIENT_ID)
    #[arg(long, env = "BOBBY_GITHUB_CLIENT_ID")]
    github_client_id: Option<String>,

    /// GitHub OAuth client secret (env: BOBBY_GITHUB_CLIENT_SECRET)
    #[arg(long, env = "BOBBY_GITHUB_CLIENT_SECRET")]
    github_client_secret: Option<String>,

    /// Session signing secret (env: BOBBY_SESSION_SECRET)
    #[arg(long, env = "BOBBY_SESSION_SECRET")]
    session_secret: Option<String>,

    /// whether Redis should be used for persistent session storage
    #[arg(long, default_value_t = false)]
    use_redis: bool,

    /// Redis URL for persistent admin session storage (env: BOBBY_REDIS_ADMIN_URL)
    #[arg(long, env = "BOBBY_REDIS_ADMIN_URL")]
    redis_admin_url: Option<String>,

    /// Comma-separated list of allowed GitHub usernames (env: BOBBY_ADMIN_USERS)
    #[arg(long, env = "BOBBY_ADMIN_USERS")]
    admin_users: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The admin-session and publish redis urls are `rediss://`, so TLS runs
    // through rustls — install the process-global crypto provider once at startup.
    #[allow(clippy::expect_used)]
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install rustls crypto provider");

    let args = Args::parse();

    let _guard = shared::tracing::init_with_file(
        "skeet_appraise=info,skeet_publish=info,shared=info,skeet_store=info",
        "appraise.log",
    );
    info!(git_hash = env!("BUILD_GIT_HASH"), "skeet-appraise starting");

    let store = Arc::new(args.store.open_store("appraise").await?);
    let models = Arc::new(RefineModels::load(&args.model_path)?);

    // The selectable feeds are discovered from the publisher's catalog on each
    // home render (not cached at startup), so feeds published after skeet-appraise
    // comes up are picked up without a restart.
    let feeds_reader = Arc::new(PublishedListCatalogReader::new(args.redis_publish_url));

    info!(bind = %args.bind, "starting skeet-appraise server");

    let appraiser = if args.local_admin {
        info!("local admin mode enabled");
        Some(Arc::new(Appraiser::LocalAdmin))
    } else {
        None
    };

    let oauth_config = match (
        args.github_client_id,
        args.github_client_secret,
        &args.admin_users,
    ) {
        (Some(client_id), Some(client_secret), Some(admin_users)) => {
            let users: Vec<String> = admin_users
                .split(',')
                .map(|s| s.trim().to_string())
                .collect();
            info!(admin_users = ?users, "GitHub OAuth configured");
            Some(Arc::new(OAuthConfig::new(client_id, client_secret, users)))
        }
        _ => {
            if !args.local_admin {
                info!("no OAuth config — admin area will be inaccessible without --local-admin");
            }
            None
        }
    };

    let project = AppraiseProject {
        published_feed_layer: PublishedFeedLayer::new(feeds_reader),
        store_layer: StoreLayer::from_shared(store),
        models_layer: ModelsLayer::from_shared(models),
        appraiser_layer: AppraiserLayer::new(appraiser),
        oauth_config_layer: OAuthConfigLayer::new(oauth_config),
        started_at_layer: StartedAtLayer::new(Utc::now()),
        session_secret: args.session_secret,
        use_redis: args.use_redis,
        redis_url: args.redis_admin_url,
    };
    let bootstrapper = Bootstrapper::new(project)
        .with_config_name("dev")?
        .boot()
        .await?;
    cot::run(bootstrapper, &args.bind).await?;
    Ok(())
}
