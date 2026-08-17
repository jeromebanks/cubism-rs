//! Milestone 10b-1 (`docs/TIMESERIES_ROADMAP.md`): the value-materialization
//! primitive `CoveragePlan` (Milestone 10) does not itself provide — pure,
//! synchronous decode+merge of `AverageState` blobs already read out of a
//! states table by `AggregateReader::read_window`. This is the "merging"
//! half of the roadmap's deferred "Milestone 10b" note
//! (`range_query.rs`'s module doc comment), narrowed to one measure kind.
//!
//! Deliberately narrow, per the roadmap split recorded in Milestone 10b's
//! own entry:
//! - **Only `AverageState`** (`AggKind::Avg`'s `Binary` blob column) —
//!   `VarianceState`/`QuantileState`/the sketch-backed kinds each need their
//!   own merge wiring and are left for a later slice once a `SeriesResponse`
//!   type actually needs to carry more than one measure kind.
//! - **No I/O.** This module never calls `AggregateReader::read_window` or
//!   touches `cubism-iceberg` — it takes already-read `RecordBatch`es as
//!   input, the same "async I/O stays in the caller" split `CoveragePlan`
//!   established for Milestone 10. `cubism-iceberg` stays a
//!   `cubism-datafusion` dev-dependency; this module does not change that.
//! - **No `SeriesResponse` type.** Building that presentation/provenance
//!   wrapper around this merge (and deciding how it carries
//!   `CoveragePlan`'s `missing`/`is_exact` fields alongside a value) is left
//!   for a successor slice ("Milestone 10b-2", not yet added to the
//!   roadmap).

use cubism_core::{AggregateState, AverageState, CubismError};
use datafusion::arrow::array::{Array, BinaryArray, LargeBinaryArray, RecordBatch};
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

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::BinaryBuilder;
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
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
}
