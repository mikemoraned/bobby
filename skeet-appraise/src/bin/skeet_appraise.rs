#![warn(clippy::all, clippy::nursery)]

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use clap::Parser;
use cot::project::Bootstrapper;
use shared::{Appraiser, RefineModels};
use skeet_appraise::auth_config::OAuthConfig;
use skeet_appraise::project::AppraiseProject;
use skeet_appraise::{
    AppraiserLayer, FeedCacheLayer, OAuthConfigLayer, StartedAtLayer, StoreLayer,
};
use skeet_publish::FeedCache;
use skeet_store::StoreArgs;
use tracing::info;

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Address to bind the server to
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: String,

    /// Maximum number of entries to show on the home page
    #[arg(long, default_value_t = 10)]
    max_entries: usize,

    /// Maximum age in hours for entries to be included
    #[arg(long, default_value_t = 48)]
    max_age_hours: u64,

    /// Path to the refine model registry (refine.toml)
    #[arg(long, default_value = "config/refine.toml")]
    model_path: PathBuf,

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

    /// Redis URL for persistent session storage (env: BOBBY_REDIS_URL)
    #[arg(long, env = "BOBBY_REDIS_URL")]
    redis_url: Option<String>,

    /// Comma-separated list of allowed GitHub usernames (env: BOBBY_ADMIN_USERS)
    #[arg(long, env = "BOBBY_ADMIN_USERS")]
    admin_users: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Installs the process-global default exactly once at startup, so it cannot already
    // be set; the `expect` keeps that failure reason explicit.
    #[allow(clippy::expect_used)]
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install rustls crypto provider");

    let args = Args::parse();

    let _guard = shared::tracing::init_with_file(
        "skeet_appraise=info,shared=info,skeet_store=info",
        "appraise.log",
    );
    info!(git_hash = env!("BUILD_GIT_HASH"), "skeet-appraise starting");

    let store = Arc::new(args.store.open_store("appraise").await?);

    let models = Arc::new(
        RefineModels::load(&args.model_path)
            .unwrap_or_else(|e| panic!("failed to load {}: {e}", args.model_path.display())),
    );
    info!(path = %args.model_path.display(), "loaded refine models");

    info!(bind = %args.bind, "starting skeet-appraise server");

    let cache = Arc::new(FeedCache::new(
        Arc::clone(&store),
        Arc::clone(&models),
        args.max_entries,
        args.max_age_hours,
    ));
    cache.spawn_background_refresh();

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
        cache_layer: FeedCacheLayer::new(cache),
        store_layer: StoreLayer::from_shared(store),
        appraiser_layer: AppraiserLayer::new(appraiser),
        oauth_config_layer: OAuthConfigLayer::new(oauth_config),
        started_at_layer: StartedAtLayer::new(Utc::now()),
        session_secret: args.session_secret,
        use_redis: args.use_redis,
        redis_url: args.redis_url,
    };
    let bootstrapper = Bootstrapper::new(project)
        .with_config_name("dev")?
        .boot()
        .await?;
    cot::run(bootstrapper, &args.bind).await?;
    Ok(())
}
