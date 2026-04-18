use cot::config::{CacheUrl, ProjectConfig, SameSite, SecretKey};
use cot::middleware::SessionMiddleware;
use cot::project::{MiddlewareContext, RegisterAppsContext, RootHandler, RootHandlerBuilder};
use cot::router::{Route, Router};
use cot::session::store::memory::MemoryStore;
use cot::session::store::redis::RedisStore;
use cot::static_files::StaticFilesMiddleware;
use cot::{App, AppBuilder, Project};
use tracing::info;

use crate::AppraiserLayer;
use crate::StoreLayer;
use crate::web_static_files;
use crate::FeedCacheLayer;
use crate::OAuthConfigLayer;
use crate::StartedAtLayer;
use crate::admin::{admin, appraise_image, appraise_skeet};
use crate::auth::{auth_callback, auth_login, auth_logout};
use crate::feed_config::FeedConfigLayer;
use crate::handlers::{
    annotated_image, describe_feed_generator, did_document, get_feed_skeleton, home,
};

pub struct FeedApp;

impl App for FeedApp {
    fn name(&self) -> &'static str {
        env!("CARGO_PKG_NAME")
    }

    fn static_files(&self) -> Vec<cot::static_files::StaticFile> {
        web_static_files()
    }

    fn router(&self) -> Router {
        Router::with_urls([
            Route::with_handler_and_name("/", home, "home"),
            Route::with_handler_and_name(
                "/.well-known/did.json",
                did_document,
                "did_document",
            ),
            Route::with_handler_and_name(
                "/xrpc/app.bsky.feed.describeFeedGenerator",
                describe_feed_generator,
                "describe_feed_generator",
            ),
            Route::with_handler_and_name(
                "/xrpc/app.bsky.feed.getFeedSkeleton",
                get_feed_skeleton,
                "get_feed_skeleton",
            ),
            Route::with_handler_and_name(
                "/skeet/{image_id}/annotated.png",
                annotated_image,
                "annotated_image",
            ),
            Route::with_handler_and_name("/admin", admin, "admin"),
            Route::with_handler_and_name(
                "/admin/appraise/skeet",
                appraise_skeet,
                "appraise_skeet",
            ),
            Route::with_handler_and_name(
                "/admin/appraise/image",
                appraise_image,
                "appraise_image",
            ),
            Route::with_handler_and_name("/auth/login", auth_login, "auth_login"),
            Route::with_handler_and_name("/auth/callback", auth_callback, "auth_callback"),
            Route::with_handler_and_name("/auth/logout", auth_logout, "auth_logout"),
        ])
    }
}

pub struct FeedProject {
    pub cache_layer: FeedCacheLayer,
    pub feed_config_layer: FeedConfigLayer,
    pub store_layer: StoreLayer,
    pub appraiser_layer: AppraiserLayer,
    pub oauth_config_layer: OAuthConfigLayer,
    pub started_at_layer: StartedAtLayer,
    pub session_secret: Option<String>,
    pub redis_url: Option<String>,
}

impl Project for FeedProject {
    fn config(&self, _config_name: &str) -> cot::Result<ProjectConfig> {
        let mut builder = ProjectConfig::builder();
        builder.debug(cfg!(debug_assertions));
        if let Some(ref secret) = self.session_secret {
            builder.secret_key(SecretKey::new(secret.as_bytes()));
        }
        Ok(builder.build())
    }

    fn register_apps(&self, apps: &mut AppBuilder, _context: &RegisterAppsContext) {
        apps.register_with_views(FeedApp, "");
    }

    fn middlewares(
        &self,
        handler: RootHandlerBuilder,
        context: &MiddlewareContext,
    ) -> RootHandler {
        let session_middleware = self.redis_url.as_ref().map_or_else(
            || {
                info!("using in-memory session store");
                SessionMiddleware::new(MemoryStore::new()).secure(false).same_site(SameSite::Lax)
            },
            |url| {
                info!("using Redis session store");
                let store = RedisStore::new(&CacheUrl::from(url.as_str()))
                    .expect("failed to create Redis session store");
                SessionMiddleware::new(store).secure(true).same_site(SameSite::Lax)
            },
        );

        handler
            .middleware(StaticFilesMiddleware::from_context(context))
            .middleware(session_middleware)
            .middleware(self.cache_layer.clone())
            .middleware(self.feed_config_layer.clone())
            .middleware(self.store_layer.clone())
            .middleware(self.appraiser_layer.clone())
            .middleware(self.oauth_config_layer.clone())
            .middleware(self.started_at_layer.clone())
            .build()
    }
}
