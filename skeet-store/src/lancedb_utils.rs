use std::time::{Duration, Instant};

use arrow_array::RecordBatch;
use futures::TryStreamExt;
use lancedb::query::ExecutableQuery;
use tracing::{debug, warn};

use crate::StoreError;
use crate::query_plan::QueryPlan;

const SLOW_QUERY_THRESHOLD: Duration = Duration::from_millis(100);

pub async fn execute_query(
    query: &(impl ExecutableQuery + Sync),
    label: &str,
) -> Result<Vec<RecordBatch>, StoreError> {
    let start = Instant::now();
    let raw_plan = query.explain_plan(true).await?;
    let batches: Vec<RecordBatch> = query.execute().await?.try_collect().await?;
    let elapsed = start.elapsed();

    let plan = QueryPlan::parse(&raw_plan);
    let table = plan.table.as_deref().unwrap_or("");
    let columns = plan.columns.as_deref().unwrap_or("");
    let num_fragments = plan.num_fragments.unwrap_or(0);
    let full_filter = plan.full_filter.as_deref().unwrap_or("");
    let full_scan = plan.full_scan();
    let index = plan.index.as_deref().unwrap_or("");
    let unknown_suffix = if plan.unknown_keys.is_empty() {
        String::new()
    } else {
        let keys: Vec<&str> = plan.unknown_keys.iter().map(String::as_str).collect();
        format!(" (unknown plan keys: {})", keys.join(", "))
    };

    if elapsed > SLOW_QUERY_THRESHOLD {
        warn!(
            %label,
            ?elapsed,
            plan.table = table,
            plan.columns = columns,
            plan.num_fragments = num_fragments,
            plan.full_scan = full_scan,
            plan.full_filter = full_filter,
            plan.index = index,
            "slow query{unknown_suffix}"
        );
    } else {
        debug!(
            %label,
            ?elapsed,
            plan.table = table,
            plan.columns = columns,
            plan.num_fragments = num_fragments,
            plan.full_scan = full_scan,
            plan.full_filter = full_filter,
            plan.index = index,
            "query{unknown_suffix}"
        );
    }

    Ok(batches)
}
