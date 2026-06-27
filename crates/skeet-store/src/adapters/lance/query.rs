use std::time::Instant;

use arrow_array::RecordBatch;
use arrow_schema::{DataType, TimeUnit};
use futures::TryStreamExt;
use lancedb::expr::{DfExpr, col, expr_cast, lit};
use lancedb::query::ExecutableQuery;

use crate::StoreError;
use crate::observability::log_query_plan;

/// Typed predicate builders — the single seam for `only_if_expr` filters.
///
/// Building filters as [`DfExpr`] trees rather than interpolating SQL strings is
/// injection-proof (values become literals, never syntax) and is the gateway to
/// engine pushdown. Keep every adapter predicate going through these helpers.
///
/// These are thin conveniences over the DataFusion expression builder
/// (`datafusion-expr`, surfaced here via `lancedb::expr` — `col`/`lit`/`expr_cast`
/// plus `Expr`'s `eq`/`in_list`/`lt`/`gt_eq` combinators); there is no dedicated
/// third-party predicate-builder crate beyond DataFusion itself. A future refactor
/// that takes a direct `datafusion` dep could drop [`utc_micros`]'s `expr_cast` in
/// favour of a `ScalarValue::TimestampMicrosecond` literal.
/// `<column> = <value>` for a string column.
pub fn col_eq(column: &str, value: impl Into<String>) -> DfExpr {
    col(column).eq(lit(value.into()))
}

/// `<column> = <value>` for an integer column.
pub fn col_eq_int(column: &str, value: i64) -> DfExpr {
    col(column).eq(lit(value))
}

/// `<column> IN (<values>)` for a string column.
pub fn col_in(column: &str, values: impl IntoIterator<Item = String>) -> DfExpr {
    col(column).in_list(values.into_iter().map(lit).collect(), false)
}

/// `<column> < <micros>` against a UTC microsecond-timestamp column.
pub fn col_before_micros(column: &str, micros: i64) -> DfExpr {
    col(column).lt(utc_micros(micros))
}

/// `<column> >= <micros>` against a UTC microsecond-timestamp column.
pub fn col_at_or_after_micros(column: &str, micros: i64) -> DfExpr {
    col(column).gt_eq(utc_micros(micros))
}

/// `<start_micros> <= <column> < <end_micros>` — a half-open window against a
/// UTC microsecond-timestamp column.
pub fn col_in_micros_range(column: &str, start_micros: i64, end_micros: i64) -> DfExpr {
    col_at_or_after_micros(column, start_micros).and(col_before_micros(column, end_micros))
}

/// A `Timestamp(Microsecond, UTC)` literal from epoch micros — the typed
/// equivalent of the `arrow_cast(n, 'Timestamp(Microsecond, Some("UTC"))')` cast.
fn utc_micros(micros: i64) -> DfExpr {
    expr_cast(
        lit(micros),
        DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
    )
}

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
