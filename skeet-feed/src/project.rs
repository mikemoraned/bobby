use cot::config::ProjectConfig;
use cot::project::{MiddlewareContext, RegisterAppsContext, RootHandler, RootHandlerBuilder};
use cot::router::{Route, Router};
use cot::static_files::StaticFilesMiddleware;
use cot::{App, AppBuilder, Project};

use skeet_web_shared::{StoreLayer, web_static_files};

use crate::AppraiserLayer;
use crate::FeedCacheLayer;
use crate::StartedAtLayer;
use crate::feed_config::FeedConfigLayer;
use crate::admin::{admin, appraise_image, appraise_skeet};
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
        ])
    }
}

pub struct FeedProject {
    pub cache_layer: FeedCacheLayer,
    pub feed_config_layer: FeedConfigLayer,
    pub store_layer: StoreLayer,
    pub appraiser_layer: AppraiserLayer,
    pub started_at_layer: StartedAtLayer,
}

impl Project for FeedProject {
    fn config(&self, _config_name: &str) -> cot::Result<ProjectConfig> {
        Ok(ProjectConfig::dev_default())
    }

    fn register_apps(&self, apps: &mut AppBuilder, _context: &RegisterAppsContext) {
        apps.register_with_views(FeedApp, "");
    }

    fn middlewares(
        &self,
        handler: RootHandlerBuilder,
        context: &MiddlewareContext,
    ) -> RootHandler {
        handler
            .middleware(StaticFilesMiddleware::from_context(context))
            .middleware(self.cache_layer.clone())
            .middleware(self.feed_config_layer.clone())
            .middleware(self.store_layer.clone())
            .middleware(self.appraiser_layer.clone())
            .middleware(self.started_at_layer.clone())
            .build()
    }
}
