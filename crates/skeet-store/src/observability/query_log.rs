use std::time::Duration;

use tracing::{debug, warn};

use shared::query_plan::QueryPlan;

const SLOW_QUERY_THRESHOLD: Duration = Duration::from_millis(100);

/// Parse a LanceDB plan and emit it as a structured `tracing` event — at `warn`
/// if the query ran slower than [`SLOW_QUERY_THRESHOLD`], otherwise at `debug`.
pub fn log_query_plan(label: &str, elapsed: Duration, raw_plan: &str) {
    let plan = QueryPlan::parse(raw_plan);
    let table = plan.table.as_deref().unwrap_or("");
    let columns = plan.columns.as_deref().unwrap_or("");
    let num_fragments = plan.num_fragments.unwrap_or(0);
    let full_filter = plan.full_filter.as_deref().unwrap_or("");
    let refine_filter = plan.refine_filter.as_deref().unwrap_or("");
    let range_before = plan.range_before.as_deref().unwrap_or("");
    let range_after = plan.range_after.as_deref().unwrap_or("");
    let full_scan = plan.full_scan();
    let index = plan.index.as_deref().unwrap_or("");

    if !plan.unknown_keys.is_empty() {
        let keys: Vec<&str> = plan.unknown_keys.iter().map(String::as_str).collect();
        warn!(
            %label,
            unknown_keys = keys.join(","),
            "lance plan has unrecognized fields"
        );
    }

    if elapsed > SLOW_QUERY_THRESHOLD {
        warn!(
            %label,
            ?elapsed,
            plan.table = table,
            plan.columns = columns,
            plan.num_fragments = num_fragments as i64,
            plan.full_scan = full_scan,
            plan.full_filter = full_filter,
            plan.refine_filter = refine_filter,
            plan.range_before = range_before,
            plan.range_after = range_after,
            plan.row_id = plan.row_id,
            plan.row_addr = plan.row_addr,
            plan.index = index,
            "slow query"
        );
    } else {
        debug!(
            %label,
            ?elapsed,
            plan.table = table,
            plan.columns = columns,
            plan.num_fragments = num_fragments as i64,
            plan.full_scan = full_scan,
            plan.full_filter = full_filter,
            plan.refine_filter = refine_filter,
            plan.range_before = range_before,
            plan.range_after = range_after,
            plan.row_id = plan.row_id,
            plan.row_addr = plan.row_addr,
            plan.index = index,
            "query"
        );
    }
}
