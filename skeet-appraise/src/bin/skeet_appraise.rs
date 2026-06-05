#![warn(clippy::all, clippy::nursery)]

use std::sync::Arc;

use chrono::Utc;
use clap::Parser;
use cot::project::Bootstrapper;
use shared::Appraiser;
use skeet_appraise::auth_config::OAuthConfig;
use skeet_appraise::project::AppraiseProject;
use skeet_appraise::{
    AppraiserLayer, OAuthConfigLayer, PublishedFeedLayer, StartedAtLayer, StoreLayer,
};
use skeet_publish::{Limit, Order, RedisFeedSource};
use skeet_store::StoreArgs;
use tracing::info;

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Address to bind the server to
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: String,

    /// Redis URL for the publish server (env: BOBBY_REDIS_PUBLISH_URL) — the
    /// home page's source of truth for what's in the feed.
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

    info!(bind = %args.bind, "starting skeet-appraise server");

    // The home page mirrors the Bluesky feed: the published `recency-48h` list.
    let feed = Arc::new(RedisFeedSource::new(
        args.redis_publish_url,
        Order::Recency,
        Limit::hours(48),
    ));

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
        published_feed_layer: PublishedFeedLayer::new(feed),
        store_layer: StoreLayer::from_shared(store),
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
