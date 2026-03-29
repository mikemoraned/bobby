use cot::config::ProjectConfig;
use cot::project::{
    MiddlewareContext, RegisterAppsContext, RootHandler, RootHandlerBuilder,
};
use cot::router::{Route, Router};
use cot::{App, AppBuilder, Project};

use crate::StoreLayer;
use crate::handlers::{annotated_image, home, pruned, refined};

pub struct InspectApp;

impl App for InspectApp {
    fn name(&self) -> &'static str {
        env!("CARGO_PKG_NAME")
    }

    fn router(&self) -> Router {
        Router::with_urls([
            Route::with_handler_and_name("/", home, "home"),
            Route::with_handler_and_name("/pruned", pruned, "pruned"),
            Route::with_handler_and_name("/refined", refined, "refined"),
            Route::with_handler_and_name(
                "/skeet/{image_id}/annotated.png",
                annotated_image,
                "annotated_image",
            ),
        ])
    }
}

pub struct InspectProject {
    pub store_layer: StoreLayer,
}

impl Project for InspectProject {
    fn config(&self, _config_name: &str) -> cot::Result<ProjectConfig> {
        Ok(ProjectConfig::dev_default())
    }

    fn register_apps(&self, apps: &mut AppBuilder, _context: &RegisterAppsContext) {
        apps.register_with_views(InspectApp, "");
    }

    fn middlewares(
        &self,
        handler: RootHandlerBuilder,
        _context: &MiddlewareContext,
    ) -> RootHandler {
        handler.middleware(self.store_layer.clone()).build()
    }
}
