use cot::config::ProjectConfig;
use cot::project::{MiddlewareContext, RegisterAppsContext, RootHandler, RootHandlerBuilder};
use cot::router::{Route, Router};
use cot::{App, AppBuilder, Project};

use crate::feed_config::FeedConfigLayer;
use crate::handlers::{describe_feed_generator, did_document, get_feed_skeleton, home};
use crate::{DimensionCacheLayer, FeedSourceLayer, PublishedImagesSourceLayer};

pub struct FeedApp;

impl App for FeedApp {
    fn name(&self) -> &'static str {
        env!("CARGO_PKG_NAME")
    }

    fn router(&self) -> Router {
        Router::with_urls([
            Route::with_handler_and_name("/", home, "home"),
            Route::with_handler_and_name("/.well-known/did.json", did_document, "did_document"),
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
        ])
    }
}

pub struct FeedProject {
    pub feed_source_layer: FeedSourceLayer,
    pub published_images_source_layer: PublishedImagesSourceLayer,
    pub dimension_cache_layer: DimensionCacheLayer,
    pub feed_config_layer: FeedConfigLayer,
}

impl Project for FeedProject {
    fn config(&self, _config_name: &str) -> cot::Result<ProjectConfig> {
        let mut builder = ProjectConfig::builder();
        builder.debug(cfg!(debug_assertions));
        Ok(builder.build())
    }

    fn register_apps(&self, apps: &mut AppBuilder, _context: &RegisterAppsContext) {
        apps.register_with_views(FeedApp, "");
    }

    fn middlewares(
        &self,
        handler: RootHandlerBuilder,
        _context: &MiddlewareContext,
    ) -> RootHandler {
        handler
            .middleware(self.feed_source_layer.clone())
            .middleware(self.published_images_source_layer.clone())
            .middleware(self.dimension_cache_layer.clone())
            .middleware(self.feed_config_layer.clone())
            .build()
    }
}
