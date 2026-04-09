use std::time::{Duration, Instant};

use arrow_array::RecordBatch;
use futures::TryStreamExt;
use lancedb::query::ExecutableQuery;
use tracing::{debug, warn};

use crate::StoreError;

const SLOW_QUERY_THRESHOLD: Duration = Duration::from_millis(100);

pub async fn execute_query(
    query: &(impl ExecutableQuery + Sync),
    label: &str,
) -> Result<Vec<RecordBatch>, StoreError> {
    let plan = query.explain_plan(true).await?;
    let start = Instant::now();
    let batches: Vec<RecordBatch> = query.execute().await?.try_collect().await?;
    let elapsed = start.elapsed();

    if elapsed > SLOW_QUERY_THRESHOLD {
        warn!(%label, ?elapsed, %plan, "slow query");
    } else {
        debug!(%label, ?elapsed, %plan, "query");
    }

    Ok(batches)
}
