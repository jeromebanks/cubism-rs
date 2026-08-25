//! Milestone 10b-1 (`docs/TIMESERIES_ROADMAP.md`): the value-materialization
//! primitive `CoveragePlan` (Milestone 10) does not itself provide — pure,
//! synchronous decode+merge of `AverageState` blobs already read out of a
//! states table by `AggregateReader::read_window`. This is the "merging"
//! half of the roadmap's deferred "Milestone 10b" note
//! (`range_query.rs`'s module doc comment), narrowed to one measure kind.
//!
//! Deliberately narrow, per the roadmap split recorded in Milestone 10b's
//! own entry:
//! - **Only `AverageState`** (`AggKind::Avg`'s `Binary` blob column) among
//!   the *blob*-backed kinds — `VarianceState`/`QuantileState`/the
//!   sketch-backed kinds each need their own decode wiring and are left
//!   for a later slice once a `SeriesResponse` type actually needs to
//!   carry more than one blob kind. Milestone 14 widened this module
//!   beside (not into) that function: the four *scalar* measure kinds
//!   (`AggKind::Count`/`Sum`/`Min`/`Max`) have no blob to decode at all —
//!   `temporal_state_schema` stores them as plain `Int64`/`Float64`
//!   columns — so [`merge_scalar_column`] folds them directly.
//! - **No I/O.** This module never calls `AggregateReader::read_window` or
//!   touches `cubism-iceberg` — it takes already-read `RecordBatch`es as
//!   input, the same "async I/O stays in the caller" split `CoveragePlan`
//!   established for Milestone 10. `cubism-iceberg` stays a
//!   `cubism-datafusion` dev-dependency; this module does not change that.
//! - **No `SeriesResponse` type.** That presentation/provenance wrapper
//!   around this merge (`CoveragePlan`'s `missing`/`is_exact` fields
//!   alongside a value) is [`crate::series_response::SeriesResponse`]
//!   (Milestone 10b-2) — not built in this module.

use cubism_core::{AggregateState, AggKind, AverageState, CubismError};
use datafusion::arrow::array::{
    Array, BinaryArray, Float64Array, Int64Array, LargeBinaryArray, RecordBatch,
};
use datafusion::arrow::datatypes::DataType;

/// Decode and merge every non-null `AverageState` blob in `column` across
/// all of `batches`, in row order, into one merged state.
///
/// Accepts `column` as either `Binary` or `LargeBinary`: `temporal_build`'s
/// `temporal_state_schema` declares `AggKind::Avg` as `DataType::Binary`,
/// but `iceberg`'s Arrow schema conversion always widens
/// `PrimitiveType::Binary` to `DataType::LargeBinary` on the way back out of
/// a scan (`iceberg::arrow::schema`'s `visit_type` for `PrimitiveType::Binary`,
/// iceberg 0.10.0) — so a batch fresh from `temporal_build` and a batch read
/// back via `AggregateReader::read_window` carry the same *bytes* in this
/// column under two different Arrow types. Both are handled here rather
/// than making a batch's caller normalize one to the other first.
///
/// Returns `AverageState::new()` (zero sum, zero count) if every value is
/// null or no rows are present at all — an empty range merges to "no
/// data," not an error, matching `AverageState::mean`'s own `None`-on-zero-
/// count behavior.
///
/// Fails with `CubismError::AggregateState` if `column` does not exist in a
/// batch's schema, is neither a `Binary` nor a `LargeBinary` array, or a
/// non-null value fails to decode as an `AverageState` (a corrupt or
/// wrong-measure-kind blob).
///
/// **No `XUnit` filtering, by design — this function stays selector-
/// agnostic.** Every non-null blob in every row of `batches` is folded in,
/// with no awareness of the row's `xunit_id`. A states table can carry rows
/// for more than one distinct lattice cell in the same window (the global
/// rollup and each per-dimension cell are separately aggregated rows, not
/// derivable from each other); this function does not distinguish them.
/// Callers must pre-filter `batches` to the rows for the cell(s) they
/// actually want before calling this — nothing here enforces that.
/// [`crate::series_response::SeriesResponse::new`] (Milestone 10b-3,
/// `docs/TIMESERIES_ROADMAP.md`) is this crate's one caller, and it does
/// exactly that pre-filtering for its single-selector case — see its own
/// doc comment for the resolve-then-filter mechanics that fixed
/// [#19](https://github.com/jeromebanks/cubism-rs/issues/19) at that call
/// site rather than here.
pub fn merge_average_column(
    batches: &[RecordBatch],
    column: &str,
) -> Result<AverageState, CubismError> {
    let mut merged = AverageState::new();
    for batch in batches {
        let index = batch.schema().index_of(column).map_err(|_| {
            CubismError::AggregateState(format!("column '{column}' not found in states batch"))
        })?;
        let column_array = batch.column(index);
        match column_array.data_type() {
            DataType::Binary => {
                let array = column_array.as_any().downcast_ref::<BinaryArray>().unwrap();
                for row in 0..array.len() {
                    if array.is_null(row) {
                        continue;
                    }
                    merged = merged.merge(&AverageState::decode(array.value(row))?)?;
                }
            }
            DataType::LargeBinary => {
                let array = column_array
                    .as_any()
                    .downcast_ref::<LargeBinaryArray>()
                    .unwrap();
                for row in 0..array.len() {
                    if array.is_null(row) {
                        continue;
                    }
                    merged = merged.merge(&AverageState::decode(array.value(row))?)?;
                }
            }
            other => {
                return Err(CubismError::AggregateState(format!(
                    "column '{column}' is {other:?}, not a Binary or LargeBinary array"
                )));
            }
        }
    }
    Ok(merged)
}

/// Merge a *scalar* measure column — [`AggKind::Count`]'s `Int64` or
/// [`AggKind::Sum`]/[`AggKind::Min`]/[`AggKind::Max`]'s `Float64`, the two
/// non-blob column types [`crate::temporal_build::temporal_state_schema`]
/// declares — across all of `batches`, in row order, into one presented
/// value. Milestone 14's widening beside [`merge_average_column`]: these
/// four kinds carry no state blob to decode, and their cross-row/
/// cross-window merge is just the aggregate's own fold — count sums,
/// sum sums, min minimizes, max maximizes — so there is nothing here for a
/// state type to do.
///
/// **"No data" is structural, not numeric.** Returns `None` when no
/// non-null value was folded in at all (no rows, all nulls, or the
/// selector filter removed every row before this call), regardless of what
/// a numeric default would suggest — in particular min-of-nothing is not
/// `0.0`. A genuine folded value of `0` (count) or `-0.0`-adjacent sums is
/// `Some(..)`; callers apply gap policy to the `None` case only.
///
/// Count folds in `i64` and converts to `f64` once at the end, so the
/// presented value stays exact up to 2^53 like every other `f64`
/// presentation (and exact far beyond that internally).
///
/// Fails with `CubismError::AggregateState` if `agg` is not one of the four
/// scalar kinds (blob-backed kinds must go through their own decode path),
/// if `column` does not exist in a batch's schema, if the column's Arrow
/// type does not match `agg`'s declared storage (`Count` merges `Int64`;
/// `Sum`/`Min`/`Max` merge `Float64` — a mismatch means the caller resolved
/// the measure name to someone else's column), or if the column is neither
/// scalar type at all.
pub fn merge_scalar_column(
    batches: &[RecordBatch],
    column: &str,
    agg: AggKind,
) -> Result<Option<f64>, CubismError> {
    if !matches!(agg, AggKind::Count | AggKind::Sum | AggKind::Min | AggKind::Max) {
        return Err(CubismError::AggregateState(format!(
            "{agg:?} is not a scalar measure kind; its states are blobs with their own decode path"
        )));
    }
    let mut seen = false;
    let mut count_total: i64 = 0;
    let mut merged: Option<f64> = None;
    for batch in batches {
        let index = batch.schema().index_of(column).map_err(|_| {
            CubismError::AggregateState(format!("column '{column}' not found in states batch"))
        })?;
        let column_array = batch.column(index);
        match (agg, column_array.data_type()) {
            (AggKind::Count, DataType::Int64) => {
                let array = column_array.as_any().downcast_ref::<Int64Array>().unwrap();
                for row in 0..array.len() {
                    if array.is_null(row) {
                        continue;
                    }
                    seen = true;
                    count_total += array.value(row);
                }
            }
            (AggKind::Sum | AggKind::Min | AggKind::Max, DataType::Float64) => {
                let array = column_array.as_any().downcast_ref::<Float64Array>().unwrap();
                for row in 0..array.len() {
                    if array.is_null(row) {
                        continue;
                    }
                    seen = true;
                    let value = array.value(row);
                    merged = Some(match agg {
                        AggKind::Sum => merged.unwrap_or(0.0) + value,
                        AggKind::Min => merged.map_or(value, |current| current.min(value)),
                        AggKind::Max => merged.map_or(value, |current| current.max(value)),
                        // Excluded by this arm's own `(agg, DataType::Float64)`
                        // pattern: `Count` folds `Int64` columns above, never
                        // here.
                        AggKind::Count => unreachable!("count folds Int64 columns"),
                        _ => unreachable!(
                            "the opening guard rejected every non-scalar kind"
                        ),
                    });
                }
            }
            (_, other) => {
                return Err(CubismError::AggregateState(format!(
                    "column '{column}' is {other:?}; a scalar measure's states column must be \
                     Int64 (count) or Float64 (sum/min/max)"
                )));
            }
        }
    }
    if !seen {
        return Ok(None);
    }
    Ok(match agg {
        AggKind::Count => Some(count_total as f64),
        _ => merged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{BinaryBuilder, ArrowPrimitiveType, PrimitiveArray};
    use datafusion::arrow::datatypes::{DataType, Float64Type, Int64Type, Field, Schema};
    use std::sync::Arc;

    fn batch_with_blobs(column: &str, blobs: &[Option<Vec<u8>>]) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![Field::new(
            column,
            DataType::Binary,
            true,
        )]));
        let mut builder = BinaryBuilder::new();
        for blob in blobs {
            match blob {
                Some(bytes) => builder.append_value(bytes),
                None => builder.append_null(),
            }
        }
        RecordBatch::try_new(schema, vec![Arc::new(builder.finish())]).unwrap()
    }

    /// Mirrors what `AggregateReader::read_window` actually hands back for
    /// this column (see this module's doc comment): `LargeBinary`, not
    /// `Binary`.
    fn large_batch_with_blobs(column: &str, blobs: &[Option<Vec<u8>>]) -> RecordBatch {
        use datafusion::arrow::array::LargeBinaryBuilder;

        let schema = Arc::new(Schema::new(vec![Field::new(
            column,
            DataType::LargeBinary,
            true,
        )]));
        let mut builder = LargeBinaryBuilder::new();
        for blob in blobs {
            match blob {
                Some(bytes) => builder.append_value(bytes),
                None => builder.append_null(),
            }
        }
        RecordBatch::try_new(schema, vec![Arc::new(builder.finish())]).unwrap()
    }

    #[test]
    fn merge_average_column_folds_multiple_batches_and_rows() {
        // Two rows in `batch1` (the shape `AggregateReader::read_window`
        // actually returns — one row per `(bucket_start, xunit_id)`, not
        // one row per window) plus a second batch, so this proves folding
        // across both rows *and* batches, not just batches.
        let mut a1 = AverageState::new();
        a1.accumulate(3.0).unwrap();
        let mut a2 = AverageState::new();
        a2.accumulate(5.0).unwrap();
        let mut b = AverageState::new();
        b.accumulate(10.0).unwrap();

        let batch1 = batch_with_blobs("avg_v1", &[Some(a1.encode()), Some(a2.encode())]);
        let batch2 = batch_with_blobs("avg_v1", &[Some(b.encode())]);

        let merged = merge_average_column(&[batch1, batch2], "avg_v1").unwrap();
        assert_eq!(merged, a1.merge(&a2).unwrap().merge(&b).unwrap());
    }

    #[test]
    fn merge_average_column_skips_nulls() {
        let mut a = AverageState::new();
        a.accumulate(7.0).unwrap();
        let batch = batch_with_blobs("avg_v1", &[Some(a.encode()), None]);

        let merged = merge_average_column(&[batch], "avg_v1").unwrap();
        assert_eq!(merged, a);
    }

    #[test]
    fn merge_average_column_empty_input_is_zero_state_not_an_error() {
        let batch = batch_with_blobs("avg_v1", &[]);
        let merged = merge_average_column(&[batch], "avg_v1").unwrap();
        assert_eq!(merged, AverageState::new());
    }

    #[test]
    fn merge_average_column_handles_large_binary_and_mixed_batches() {
        let mut a = AverageState::new();
        a.accumulate(3.0).unwrap();
        let mut b = AverageState::new();
        b.accumulate(10.0).unwrap();

        // One batch shaped like `temporal_build`'s own output (`Binary`),
        // one shaped like `AggregateReader::read_window`'s (`LargeBinary`)
        // — both must fold into the same merge.
        let batch1 = batch_with_blobs("avg_v1", &[Some(a.encode())]);
        let batch2 = large_batch_with_blobs("avg_v1", &[Some(b.encode())]);

        let merged = merge_average_column(&[batch1, batch2], "avg_v1").unwrap();
        assert_eq!(merged, a.merge(&b).unwrap());
    }

    #[test]
    fn merge_average_column_rejects_missing_column() {
        let batch = batch_with_blobs("avg_v1", &[]);
        let err = merge_average_column(&[batch], "not_here").unwrap_err();
        assert!(matches!(err, CubismError::AggregateState(_)));
    }

    #[test]
    fn merge_average_column_rejects_non_binary_column() {
        use datafusion::arrow::array::Int64Array;
        let schema = Arc::new(Schema::new(vec![Field::new(
            "avg_v1",
            DataType::Int64,
            false,
        )]));
        let array = Int64Array::from(vec![1]);
        let batch = RecordBatch::try_new(schema, vec![Arc::new(array)]).unwrap();

        let err = merge_average_column(&[batch], "avg_v1").unwrap_err();
        assert!(matches!(err, CubismError::AggregateState(_)));
    }

    fn scalar_batch<T: ArrowPrimitiveType>(column: &str, values: &[Option<T::Native>]) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![Field::new(
            column,
            T::DATA_TYPE,
            true,
        )]));
        let array = PrimitiveArray::<T>::from_iter(values.iter().copied());
        RecordBatch::try_new(schema, vec![Arc::new(array)]).unwrap()
    }

    #[test]
    fn merge_scalar_column_sums_count_across_batches_rows_and_skips_nulls() {
        // Same two-rows-plus-second-batch shape
        // `merge_average_column_folds_multiple_batches_and_rows` proves on:
        // rows within a batch AND across batches all fold, nulls skip.
        // 3 + 4 + 10 = 17.
        let batch1 = scalar_batch::<Int64Type>("views_v1", &[Some(3), Some(4)]);
        let batch2 = scalar_batch::<Int64Type>("views_v1", &[None, Some(10)]);

        let merged = merge_scalar_column(&[batch1, batch2], "views_v1", AggKind::Count).unwrap();
        assert_eq!(merged, Some(17.0));
    }

    #[test]
    fn merge_scalar_column_min_takes_minimum_and_max_takes_maximum_across_batches() {
        // min-of-mins / max-of-maxes: each batch's rows fold pairwise into
        // one running extreme, so per-batch orderings cannot leak through.
        let batch1 = scalar_batch::<Float64Type>("latency_v1", &[Some(30.0), Some(7.5)]);
        let batch2 = scalar_batch::<Float64Type>("latency_v1", &[Some(12.0)]);

        let merged = merge_scalar_column(&[batch1, batch2], "latency_v1", AggKind::Min).unwrap();
        assert_eq!(merged, Some(7.5));

        let batch1 = scalar_batch::<Float64Type>("latency_v1", &[Some(30.0), Some(7.5)]);
        let batch2 = scalar_batch::<Float64Type>("latency_v1", &[Some(12.0)]);
        let merged = merge_scalar_column(&[batch1, batch2], "latency_v1", AggKind::Max).unwrap();
        assert_eq!(merged, Some(30.0));
    }

    #[test]
    fn merge_scalar_column_sum_folds_float_values_including_negatives() {
        let batch1 = scalar_batch::<Float64Type>("revenue_v1", &[Some(-2.5), None]);
        let batch2 = scalar_batch::<Float64Type>("revenue_v1", &[Some(10.0), Some(0.25)]);

        let merged = merge_scalar_column(&[batch1, batch2], "revenue_v1", AggKind::Sum).unwrap();
        assert_eq!(merged, Some(7.75));
    }

    #[test]
    fn merge_scalar_column_no_non_null_value_is_none_not_a_numeric_default() {
        // Structural no-data: empty input AND all-null input must both be
        // `None` — in particular min-of-nothing is not 0.0, which is why
        // the flag is a seen-flag rather than an accumulator default.
        let empty = scalar_batch::<Float64Type>("latency_v1", &[]);
        assert_eq!(
            merge_scalar_column(&[empty], "latency_v1", AggKind::Min).unwrap(),
            None
        );
        let all_null = scalar_batch::<Int64Type>("views_v1", &[None, None]);
        assert_eq!(
            merge_scalar_column(&[all_null], "views_v1", AggKind::Count).unwrap(),
            None
        );
    }

    #[test]
    fn merge_scalar_column_genuine_zero_count_is_some_not_no_data() {
        // The other half of the seen-flag contract: a real folded zero is
        // `Some(0.0)`, distinguishable from the `None` case above.
        let batch = scalar_batch::<Int64Type>("views_v1", &[Some(0)]);
        assert_eq!(
            merge_scalar_column(&[batch], "views_v1", AggKind::Count).unwrap(),
            Some(0.0)
        );
    }

    #[test]
    fn merge_scalar_column_rejects_non_scalar_kind() {
        let batch = scalar_batch::<Int64Type>("views_v1", &[Some(1)]);
        let err = merge_scalar_column(&[batch], "views_v1", AggKind::Avg).unwrap_err();
        assert!(matches!(err, CubismError::AggregateState(_)));
    }

    #[test]
    fn merge_scalar_column_rejects_wrong_storage_type_for_the_kind() {
        // A count measure's column is Int64 by temporal_state_schema's
        // declaration; finding Binary means the caller resolved the measure
        // name to someone else's column. Same for a Float64 kind against an
        // Int64 column. Neither may silently fold.
        let int_batch = scalar_batch::<Int64Type>("views_v1", &[Some(1)]);
        let err = merge_scalar_column(&[int_batch], "views_v1", AggKind::Min).unwrap_err();
        assert!(matches!(err, CubismError::AggregateState(_)));

        let blob_schema = Arc::new(Schema::new(vec![Field::new(
            "views_v1",
            DataType::Binary,
            true,
        )]));
        let blob_batch = RecordBatch::try_new(
            blob_schema,
            vec![Arc::new(BinaryArray::from_iter_values([b"x" as &[u8]]))],
        )
        .unwrap();
        let err = merge_scalar_column(&[blob_batch], "views_v1", AggKind::Count).unwrap_err();
        assert!(matches!(err, CubismError::AggregateState(_)));
    }
}
