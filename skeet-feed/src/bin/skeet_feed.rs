#![warn(clippy::all, clippy::nursery)]

use std::sync::Arc;

use chrono::Utc;
use clap::Parser;
use cot::project::Bootstrapper;
use shared::Appraiser;
use skeet_feed::auth_config::OAuthConfig;
use skeet_feed::{AppraiserLayer, OAuthConfigLayer, StartedAtLayer, StoreLayer};
use skeet_feed::feed_cache::{FeedCache, FeedCacheLayer};
use skeet_feed::feed_config::{FeedConfigLayer, FeedParams};
use skeet_feed::project::FeedProject;
use skeet_store::StoreArgs;
use tracing::info;

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Hostname for the feed generator (used in DID and service endpoint)
    #[arg(long)]
    hostname: String,

    /// DID of the Bluesky account that published the feed
    #[arg(long)]
    publisher_did: String,

    /// Feed name identifier (used in the feed AT-URI)
    #[arg(long, default_value = "bobby-dev")]
    feed_name: String,

    /// Address to bind the server to
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: String,

    /// Maximum number of posts to return in the feed
    #[arg(long, default_value_t = 10)]
    max_entries: usize,

    /// Maximum age in hours for posts to be included
    #[arg(long, default_value_t = 48)]
    max_age_hours: u64,

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

    /// Redis URL for persistent session storage (env: BOBBY_REDIS_URL)
    #[arg(long, env = "BOBBY_REDIS_URL")]
    redis_url: Option<String>,

    /// Comma-separated list of allowed GitHub usernames (env: BOBBY_ADMIN_USERS)
    #[arg(long, env = "BOBBY_ADMIN_USERS")]
    admin_users: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let _guard = shared::tracing::init_with_file(
        "skeet_feed=info,shared=info,skeet_store=info",
        "feed.log",
    );

    let store = Arc::new(
        args.store
            .open_store()
            .await
            .expect("failed to open store at startup"),
    );

    let feed_params = FeedParams {
        hostname: args.hostname.clone(),
        publisher_did: args.publisher_did,
        feed_name: args.feed_name,
        max_entries: args.max_entries,
        max_age_hours: args.max_age_hours,
    };

    info!(
        bind = %args.bind,
        hostname = %args.hostname,
        feed_uri = %feed_params.feed_uri(),
        "starting skeet-feed server"
    );

    let cache = Arc::new(FeedCache::new(
        Arc::clone(&store),
        feed_params.max_entries,
        feed_params.max_age_hours,
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
            let users: Vec<String> = admin_users.split(',').map(|s| s.trim().to_string()).collect();
            info!(admin_users = ?users, "GitHub OAuth configured");
            Some(Arc::new(OAuthConfig::new(
                client_id,
                client_secret,
                users,
            )))
        }
        _ => {
            if !args.local_admin {
                info!("no OAuth config — admin area will be inaccessible without --local-admin");
            }
            None
        }
    };

    let project = FeedProject {
        cache_layer: FeedCacheLayer::new(cache),
        feed_config_layer: FeedConfigLayer::new(feed_params),
        store_layer: StoreLayer::from_shared(store),
        appraiser_layer: AppraiserLayer::new(appraiser),
        oauth_config_layer: OAuthConfigLayer::new(oauth_config),
        started_at_layer: StartedAtLayer::new(Utc::now()),
        session_secret: args.session_secret,
        redis_url: args.redis_url,
    };
    let bootstrapper = Bootstrapper::new(project)
        .with_config_name("dev")?
        .boot()
        .await?;
    cot::run(bootstrapper, &args.bind).await?;
    Ok(())
}
