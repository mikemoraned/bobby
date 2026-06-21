use std::time::Instant;

use arrow_array::RecordBatch;
use futures::TryStreamExt;
use lancedb::query::ExecutableQuery;

use crate::StoreError;
use crate::observability::log_query_plan;

pub async fn execute_query(
    query: &(impl ExecutableQuery + Sync),
    label: &str,
) -> Result<Vec<RecordBatch>, StoreError> {
    let start = Instant::now();
    let raw_plan = query.explain_plan(true).await?;
    let batches: Vec<RecordBatch> = query.execute().await?.try_collect().await?;
    log_query_plan(label, start.elapsed(), &raw_plan);
    Ok(batches)
}
