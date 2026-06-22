//! Cross-cutting observability: OTel gauges over the Lance tables and structured
//! logging of query plans. Sits on top of the adapter — it observes it.

mod query_log;
pub mod store_metrics;

pub use query_log::log_query_plan;
pub use store_metrics::StoreMetrics;
