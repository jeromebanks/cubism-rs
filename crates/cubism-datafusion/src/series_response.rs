//! Milestone 10b-2 (`docs/TIMESERIES_ROADMAP.md`): `SeriesResponse`, the
//! value/presentation wrapper the roadmap's deferred "Milestone 10b" note
//! described — wiring [`crate::series_merge::merge_average_column`]
//! (Milestone 10b-1) to a [`CoveragePlan`]'s (Milestone 10) per-segment
//! coverage so a query actually gets an answered value, not just provenance.
//!
//! Deliberately narrow, per the roadmap split recorded in this milestone's
//! own entry (same precedent Milestone 10b-1 set splitting off of Milestone
//! 10b):
//! - **Average blobs plus the four scalar measure kinds.** Milestone 10b's
//!   original scope was `AverageState` only, via
//!   [`crate::series_merge::merge_average_column`]. Milestone 14 widened
//!   that consciously: the scalar kinds (`AggKind::Count`/`Sum`/`Min`/
//!   `Max`) have plain `Int64`/`Float64` states columns rather than blobs,
//!   so [`crate::series_merge::merge_scalar_column`] folds them with no
//!   decode step, dispatched on the caller-supplied `AggKind`. The
//!   blob-backed kinds beyond avg (`VarianceState`/`QuantileState`/the
//!   sketch-backed kinds) stay deferred — each needs its own
//!   decode-and-present wiring.
//! - **No I/O.** Same "async stays in the caller" split `CoveragePlan` and
//!   `merge_average_column` both established: `SeriesResponse::new` takes
//!   already-read `RecordBatch`es per segment, one list per
//!   `coverage.segments` entry in the same order — mirroring
//!   `CoveragePlan::new`'s own `windows` parameter contract exactly,
//!   including its length-mismatch rejection. `cubism-iceberg` stays a
//!   `cubism-datafusion` dev-dependency; this module does not change that.
//! - **No per-point state/error metadata.** A decode/merge failure for any
//!   one segment's batches fails the whole `SeriesResponse::new` call via
//!   `?`, rather than producing a partial response with an error marker on
//!   just that point. The plan's "state/error metadata where appropriate"
//!   (line 716) is not implemented at that granularity this slice.
//! - **Exactly one `XUnit` selector.** Milestone 10b-3
//!   (`docs/TIMESERIES_ROADMAP.md`) fixed [#19](https://github.com/jeromebanks/cubism-rs/issues/19)
//!   by resolving the query's single selector to its `XUnitContentId` and
//!   filtering every segment's `batches` to matching rows before merging —
//!   see `SeriesResponse::new`'s own doc comment. A `selectors` slice whose
//!   length is not exactly 1 is rejected: the "multi-XUnit/multi-measure
//!   response shape" is a separate, still-unresolved roadmap decision
//!   (`docs/TIMESERIES_ROADMAP.md`'s "Phase 5 done condition", the
//!   "Unresolved decisions" bullet), not something this milestone guesses
//!   an answer for.
//!
//! **`is_exact` is inherited from `CoveragePlan`, not re-verified against
//! `batches`.** `SegmentCoverage::is_exact()` reflects only the caller-
//! supplied `published`/`missing` window lists (`range_query.rs`'s own
//! module doc comment: that mapping is a caller-supplied input this crate
//! deliberately does not derive). `SeriesResponse::new` does not and
//! cannot check that `batches[i]` actually contains rows for the windows
//! `coverage.segments[i]` reports as published — that correspondence is
//! entirely the caller's responsibility. A caller that reports a window as
//! published but hands an empty or wrong batch list still gets
//! `is_exact: true` back, with `value: None` (or `Some(0.0)` under
//! [`GapPolicy::Zero`]) — an exact-looking point with no data behind it.
//! This is not a bug this module can fix without inventing a
//! windows-to-batches correspondence it has no way to derive; it is a
//! contract callers must uphold.
//!
//! **Gap policy is consumed, not deferred.** A segment with zero published
//! windows (or whose batches hold no row for the resolved selector, or only
//! null measure values) merges to "no data" — `AverageState::new()`'s
//! `present()` is `None` for avg; the scalar fold's seen-flag yields `None`
//! for count/sum/min/max — so "no data" and "a real zero" are already
//! distinguished before `gap_policy` ever applies. `gap_policy` only
//! matters for that `None` case: [`GapPolicy::Missing`] (the default a
//! caller should pick when unsure) leaves it `None`; [`GapPolicy::Zero`]
//! substitutes `Some(0.0)`. A segment with a genuine merged value is never
//! touched by this substitution, gap policy or not.

use cubism_core::encoding::canonical_xunit_content_id;
use cubism_core::{
    AggKind, AggregateState, CanonicalXUnit, CubismError, EventTime, Resolution, WindowId,
    WindowRevision, XUnit, XUnitContentId,
};
use datafusion::arrow::array::{Array, BooleanArray, FixedSizeBinaryArray, RecordBatch};
use datafusion::arrow::compute::filter_record_batch;

use crate::range_query::{CoveragePlan, GapPolicy};
use crate::series_merge::{merge_average_column, merge_scalar_column};

/// One segment of a [`SeriesResponse`], carrying the same provenance a
/// [`crate::range_query::SegmentCoverage`] does plus a materialized value.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesPoint {
    pub bucket_start: EventTime,
    pub bucket_end: EventTime,
    /// Copied from `SegmentCoverage::is_exact()` — not re-verified against
    /// `batches`. See the module doc comment: this is `true` whenever the
    /// caller's `CoveragePlan` says the segment is aligned and fully
    /// published, even if `batches` for that segment is empty or wrong.
    pub is_exact: bool,
    /// The segment's merged value, presented per measure kind: avg presents
    /// `AggregateState::present` (the mean); count/sum/min/max present the
    /// scalar fold. `None` means no data was folded in for this segment —
    /// either no windows were published or `gap_policy` is
    /// [`GapPolicy::Missing`]; see the module doc comment. Counts are exact
    /// to 2^53 in this `f64` presentation.
    pub value: Option<f64>,
    pub published: Vec<(WindowId, WindowRevision)>,
    pub missing: Vec<WindowId>,
}

/// A [`CoveragePlan`] with a materialized value per segment. See the module
/// doc comment for scope.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesResponse {
    pub source_resolution: Resolution,
    pub points: Vec<SeriesPoint>,
}

impl SeriesResponse {
    /// `batches` must have exactly one entry per `coverage.segments`, in the
    /// same order — each entry is the already-read `RecordBatch`es for that
    /// segment's published windows (same shape `AggregateReader::read_window`
    /// returns, concatenated across every published window backing the
    /// segment). Mirrors `CoveragePlan::new`'s own `windows` parameter
    /// contract, including the length-mismatch rejection.
    ///
    /// `agg` selects the merge/present path: `AggKind::Avg` decodes and
    /// merges `AverageState` blobs from `column`; the scalar kinds
    /// (`Count`/`Sum`/`Min`/`Max`) fold the plain scalar column. Any other
    /// kind is rejected — blob-backed decode wiring beyond avg is not
    /// built (see the module doc comment). The caller must pass the same
    /// kind the column was built for: a mismatch surfaces as
    /// `CubismError::AggregateState` from the merge itself (wrong storage
    /// type for the kind), not as a silent zero.
    ///
    /// `selectors` must contain exactly one `XUnit` — anything else is
    /// rejected with `CubismError::Temporal` (see the module doc comment's
    /// "Exactly one `XUnit` selector" bullet). The selector is resolved to
    /// its `XUnitContentId` via `CanonicalXUnit::from` +
    /// `canonical_xunit_content_id` — the same two calls the build side's
    /// `cubism_xunit_content_id` UDF makes over a states row's decoded
    /// `xunit_id` (`crates/cubism-datafusion/src/temporal_build.rs`'s
    /// `XUnitContentIdUdf::invoke_with_args`), so a query-side selector and a
    /// build-side row resolve to byte-identical ids for the same logical
    /// cell. Every segment's `batches` are then filtered to only the rows
    /// whose `xunit_id` column matches that id (a null `xunit_id` matches
    /// nothing) before `merge_average_column` runs — fixes
    /// [#19](https://github.com/jeromebanks/cubism-rs/issues/19)'s silent
    /// over-merge across lattice cells for the single-selector case this
    /// milestone (10b-3) supports. One consequence worth stating plainly: a
    /// segment whose batches contain rows for other cells but none for the
    /// resolved selector now merges to "no data" (`value: None` or
    /// `Some(0.0)` under `GapPolicy::Zero`) with `is_exact` still `true` if
    /// `CoveragePlan` reports the segment fully published — the same
    /// already-documented `is_exact`-is-not-re-verified-against-`batches`
    /// caveat above, now also reachable via a selector that matches zero
    /// rows in an otherwise-published segment.
    pub fn new(
        coverage: &CoveragePlan,
        gap_policy: GapPolicy,
        agg: AggKind,
        column: &str,
        selectors: &[XUnit],
        batches: &[Vec<RecordBatch>],
    ) -> Result<Self, CubismError> {
        if batches.len() != coverage.segments.len() {
            return Err(CubismError::Temporal(format!(
                "expected one batch list per segment ({} segments, {} entries)",
                coverage.segments.len(),
                batches.len()
            )));
        }
        let [selector] = selectors else {
            return Err(CubismError::Temporal(format!(
                "SeriesResponse::new requires exactly one XUnit selector (the multi-XUnit \
                 response shape is an unresolved roadmap decision, not yet supported); got {}",
                selectors.len()
            )));
        };
        let content_id = canonical_xunit_content_id(&CanonicalXUnit::from(selector))?;

        let points = coverage
            .segments
            .iter()
            .zip(batches)
            .map(|(segment, segment_batches)| {
                let filtered = filter_batches_by_xunit(segment_batches, &content_id)?;
                let value = merged_value(&filtered, agg, column, gap_policy)?;
                Ok(SeriesPoint {
                    bucket_start: segment.segment.range.start(),
                    bucket_end: segment.segment.range.end(),
                    is_exact: segment.is_exact(),
                    value,
                    published: segment.published.clone(),
                    missing: segment.missing.clone(),
                })
            })
            .collect::<Result<Vec<_>, CubismError>>()?;

        Ok(Self {
            source_resolution: coverage.resolution,
            points,
        })
    }
}

/// Filters `batches` down to the rows whose `xunit_id` column equals
/// `content_id`'s bytes; a null `xunit_id` matches nothing. Each batch must
/// carry an `xunit_id: FixedSizeBinary(32)` column (`temporal_state_schema`'s
/// and `AggregateReader::read_window`'s shape) — fails with
/// `CubismError::AggregateState` otherwise, mirroring
/// `merge_average_column`'s own column-shape error handling.
fn filter_batches_by_xunit(
    batches: &[RecordBatch],
    content_id: &XUnitContentId,
) -> Result<Vec<RecordBatch>, CubismError> {
    let target: &[u8] = content_id.as_bytes();
    batches
        .iter()
        .map(|batch| {
            let index = batch.schema().index_of("xunit_id").map_err(|_| {
                CubismError::AggregateState("column 'xunit_id' not found in states batch".into())
            })?;
            let array = batch
                .column(index)
                .as_any()
                .downcast_ref::<FixedSizeBinaryArray>()
                .ok_or_else(|| {
                    CubismError::AggregateState(
                        "column 'xunit_id' is not a FixedSizeBinary(32) array".into(),
                    )
                })?;
            let mask = BooleanArray::from_iter(
                (0..array.len()).map(|row| Some(!array.is_null(row) && array.value(row) == target)),
            );
            filter_record_batch(batch, &mask)
                .map_err(|e| CubismError::AggregateState(e.to_string()))
        })
        .collect()
}

/// Merge `filtered` to one presented value per measure kind and apply
/// `gap_policy` to the no-data case only. Avg decodes/merges
/// `AverageState` blobs (`merge_average_column`'s zero-count state is the
/// no-data signal); the scalar kinds fold via `merge_scalar_column`
/// (whose seen-flag is). Blob-backed kinds beyond avg are rejected here —
/// the same "not guessed at" boundary the module doc comment records.
fn merged_value(
    filtered: &[RecordBatch],
    agg: AggKind,
    column: &str,
    gap_policy: GapPolicy,
) -> Result<Option<f64>, CubismError> {
    let merged = match agg {
        AggKind::Avg => merge_average_column(filtered, column)?.present(),
        AggKind::Count | AggKind::Sum | AggKind::Min | AggKind::Max => {
            merge_scalar_column(filtered, column, agg)?
        }
        other => {
            return Err(CubismError::Temporal(format!(
                "SeriesResponse does not support {other:?} measures (blob-backed kinds beyond \
                 avg are deferred, not yet wired)"
            )));
        }
    };
    Ok(match (merged, gap_policy) {
        (None, GapPolicy::Zero) => Some(0.0),
        (value, _) => value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::range_query::{ResolutionPlan, ResolutionSegment, TemporalQuery};
    use cubism_core::{
        AllowedLateness, AverageState, BucketOrigin, FixedResolution, TemporalSpec, TimeRange,
        YPath,
    };
    use datafusion::arrow::array::{
        ArrowPrimitiveType, BinaryBuilder, FixedSizeBinaryBuilder, PrimitiveArray,
    };
    use datafusion::arrow::datatypes::{DataType, Field, Float64Type, Int64Type, Schema};
    use std::sync::Arc;

    fn window(id: &str) -> WindowId {
        WindowId::new(id).unwrap()
    }

    fn revision(value: u64) -> WindowRevision {
        WindowRevision::new(value).unwrap()
    }

    /// The `XUnitContentId` a selector resolves to — computed via the same
    /// `CanonicalXUnit::from` + `canonical_xunit_content_id` pair
    /// `SeriesResponse::new` itself calls, so a test batch tagged with this
    /// id is indistinguishable from one a real build produced for the same
    /// logical cell (`temporal_build.rs`'s `XUnitContentIdUdf` computes the
    /// same two calls over a decoded `xunit_id`).
    fn content_id(xunit: &XUnit) -> [u8; 32] {
        *canonical_xunit_content_id(&CanonicalXUnit::from(xunit))
            .unwrap()
            .as_bytes()
    }

    fn global_id() -> [u8; 32] {
        content_id(&XUnit::global())
    }

    fn avg_batch(column: &str, xunit_id: [u8; 32], blobs: &[Option<Vec<u8>>]) -> RecordBatch {
        avg_batch_multi(
            column,
            &blobs
                .iter()
                .map(|b| (xunit_id, b.clone()))
                .collect::<Vec<_>>(),
        )
    }

    /// Like `avg_batch`, but each row carries its own `xunit_id` — needed to
    /// build a batch spanning more than one lattice cell for the filtering
    /// tests below.
    fn avg_batch_multi(column: &str, rows: &[([u8; 32], Option<Vec<u8>>)]) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("xunit_id", DataType::FixedSizeBinary(32), true),
            Field::new(column, DataType::Binary, true),
        ]));
        // `FixedSizeBinaryArray::try_from_iter` errors on an empty iterator
        // (can't infer the byte width from zero elements), unlike a builder
        // seeded with an explicit width — several tests below build a
        // zero-row batch, so use `FixedSizeBinaryBuilder` instead.
        let mut id_builder = FixedSizeBinaryBuilder::new(32);
        for (id, _) in rows {
            id_builder.append_value(id).unwrap();
        }
        let mut builder = BinaryBuilder::new();
        for (_, blob) in rows {
            match blob {
                Some(bytes) => builder.append_value(bytes),
                None => builder.append_null(),
            }
        }
        RecordBatch::try_new(
            schema,
            vec![Arc::new(id_builder.finish()), Arc::new(builder.finish())],
        )
        .unwrap()
    }

    fn hour() -> Resolution {
        Resolution::Fixed(FixedResolution::from_micros(3_600_000_000).unwrap())
    }

    /// A batch whose measure column is a plain scalar (`Int64` for count
    /// measures, `Float64` for sum/min/max), every row carrying the same
    /// `xunit_id` — the shape `temporal_state_schema` declares for those
    /// kinds and `AggregateReader::read_window` hands back.
    fn scalar_batch<T>(
        column: &str,
        xunit_id: [u8; 32],
        values: &[Option<T::Native>],
    ) -> RecordBatch
    where
        T: ArrowPrimitiveType,
    {
        scalar_batch_multi::<T>(
            column,
            &values.iter().map(|v| (xunit_id, *v)).collect::<Vec<_>>(),
        )
    }

    /// A one-segment `CoveragePlan`, aligned and fully published against a
    /// single window `w1` — the shared precondition for the tests below,
    /// which vary only the batches/gap_policy fed into `SeriesResponse::new`.
    fn one_segment_coverage_plan() -> CoveragePlan {
        let spec = TemporalSpec {
            event_time: "ts".into(),
            ingestion_time: None,
            base_resolution: hour(),
            origin: BucketOrigin::default(),
            timezone: "UTC".into(),
            allowed_lateness: AllowedLateness::from_micros(0).unwrap(),
            rollups: vec![],
            retention: None,
        };
        let query = TemporalQuery::new(
            "web_events",
            vec![XUnit::global()],
            vec!["avg_v1".into()],
            EventTime::from_unix_micros(0),
            EventTime::from_unix_micros(3_600_000_000),
            Some(hour()),
            false,
            GapPolicy::Missing,
            &spec,
        )
        .unwrap();
        let plan = ResolutionPlan::new(&query, &spec).unwrap();
        let windows = vec![vec![(window("w1"), Some(revision(1)))]];
        CoveragePlan::new(&plan, false, &windows).unwrap()
    }

    #[test]
    fn series_response_materializes_one_exact_published_segment() {
        let coverage = one_segment_coverage_plan();
        let mut a = AverageState::new();
        a.accumulate(3.0).unwrap();
        a.accumulate(5.0).unwrap();
        let batches = vec![vec![avg_batch("avg_v1", global_id(), &[Some(a.encode())])]];

        let response = SeriesResponse::new(
            &coverage,
            GapPolicy::Missing,
            AggKind::Avg,
            "avg_v1",
            &[XUnit::global()],
            &batches,
        )
        .unwrap();
        assert_eq!(response.source_resolution, hour());
        assert_eq!(response.points.len(), 1);
        let point = &response.points[0];
        assert!(point.is_exact);
        assert_eq!(point.value, Some(4.0));
        assert_eq!(point.published, vec![(window("w1"), revision(1))]);
        assert!(point.missing.is_empty());
        assert_eq!(point.bucket_start, EventTime::from_unix_micros(0));
        assert_eq!(point.bucket_end, EventTime::from_unix_micros(3_600_000_000));
    }

    #[test]
    fn series_response_no_data_is_none_under_missing_gap_policy() {
        let coverage = one_segment_coverage_plan();
        // No blobs at all for the segment's one published window's batch —
        // an empty read result, not an error. `coverage`'s own `missing`
        // list is still empty (the caller reported w1 as published), so
        // `is_exact` stays `true` here even though no data was actually
        // folded in — the module doc comment's documented caveat that
        // `is_exact` is inherited from `CoveragePlan`, not re-verified
        // against `batches`. Asserted explicitly so this incoherent-looking
        // combination is visible, not implied.
        let batches = vec![vec![avg_batch("avg_v1", global_id(), &[])]];

        let response = SeriesResponse::new(
            &coverage,
            GapPolicy::Missing,
            AggKind::Avg,
            "avg_v1",
            &[XUnit::global()],
            &batches,
        )
        .unwrap();
        assert!(
            response.points[0].is_exact,
            "is_exact reflects CoveragePlan's own published/missing lists, not batches content"
        );
        assert_eq!(response.points[0].value, None);
    }

    #[test]
    fn series_response_no_data_is_zero_under_zero_gap_policy() {
        let coverage = one_segment_coverage_plan();
        let batches = vec![vec![avg_batch("avg_v1", global_id(), &[])]];

        let response = SeriesResponse::new(
            &coverage,
            GapPolicy::Zero,
            AggKind::Avg,
            "avg_v1",
            &[XUnit::global()],
            &batches,
        )
        .unwrap();
        assert!(
            response.points[0].is_exact,
            "same caveat as the Missing-policy test above: is_exact does not depend on batches"
        );
        assert_eq!(response.points[0].value, Some(0.0));
    }

    #[test]
    fn series_response_gap_policy_does_not_touch_a_real_present_value() {
        // A genuine non-zero-count merge must come through unchanged
        // regardless of gap_policy — GapPolicy::Zero only substitutes for
        // the `None` (zero-count) case, never overrides a real value.
        let coverage = one_segment_coverage_plan();
        let mut a = AverageState::new();
        a.accumulate(3.0).unwrap();
        let batches = vec![vec![avg_batch("avg_v1", global_id(), &[Some(a.encode())])]];

        let response = SeriesResponse::new(
            &coverage,
            GapPolicy::Zero,
            AggKind::Avg,
            "avg_v1",
            &[XUnit::global()],
            &batches,
        )
        .unwrap();
        assert_eq!(response.points[0].value, Some(3.0));
    }

    #[test]
    fn series_response_rejects_batches_length_mismatch() {
        let coverage = one_segment_coverage_plan();
        let err = SeriesResponse::new(
            &coverage,
            GapPolicy::Missing,
            AggKind::Avg,
            "avg_v1",
            &[XUnit::global()],
            &[],
        )
        .expect_err("one segment but zero batch-list entries must be rejected");
        assert!(matches!(err, CubismError::Temporal(_)));
    }

    #[test]
    fn series_response_rejects_multi_selector_query() {
        // The multi-XUnit response shape is a separate, still-unresolved
        // roadmap decision (see the module doc comment) — Milestone 10b-3
        // rejects it explicitly rather than guessing which cell(s) to merge.
        let coverage = one_segment_coverage_plan();
        let batches = vec![vec![avg_batch("avg_v1", global_id(), &[])]];
        let err = SeriesResponse::new(
            &coverage,
            GapPolicy::Missing,
            AggKind::Avg,
            "avg_v1",
            &[XUnit::global(), XUnit::global()],
            &batches,
        )
        .expect_err("more than one selector must be rejected, not silently merged");
        assert!(matches!(err, CubismError::Temporal(_)));

        let err = SeriesResponse::new(
            &coverage,
            GapPolicy::Missing,
            AggKind::Avg,
            "avg_v1",
            &[],
            &batches,
        )
        .expect_err("zero selectors must be rejected too, not treated as \"no filter\"");
        assert!(matches!(err, CubismError::Temporal(_)));
    }

    #[test]
    fn series_response_filters_batches_to_the_resolved_selector_before_merging() {
        // The #19 regression: a segment's batch carries rows for two
        // distinct lattice cells (global and one `device=mobile` cell, both
        // real `XUnitContentId`s, not sentinel bytes) in the same window.
        // Only the row(s) matching the query's selector must be merged.
        let coverage = one_segment_coverage_plan();
        let mobile = XUnit::new(vec![
            YPath::new("device").with_attribute("device", "mobile"),
        ]);

        let mut global_state = AverageState::new();
        global_state.accumulate(3.0).unwrap();
        global_state.accumulate(5.0).unwrap();
        let mut mobile_state = AverageState::new();
        mobile_state.accumulate(1000.0).unwrap();

        let batches = vec![vec![avg_batch_multi(
            "avg_v1",
            &[
                (global_id(), Some(global_state.encode())),
                (content_id(&mobile), Some(mobile_state.encode())),
            ],
        )]];

        let response = SeriesResponse::new(
            &coverage,
            GapPolicy::Missing,
            AggKind::Avg,
            "avg_v1",
            &[XUnit::global()],
            &batches,
        )
        .unwrap();
        assert_eq!(
            response.points[0].value,
            Some(4.0),
            "must merge only the global cell's row (mean of 3.0/5.0), not the mobile cell's \
             1000.0 too"
        );
    }

    /// Like [`scalar_batch`], but each row carries its own `xunit_id` —
    /// needed to build a batch spanning more than one lattice cell.
    fn scalar_batch_multi<T>(column: &str, rows: &[([u8; 32], Option<T::Native>)]) -> RecordBatch
    where
        T: ArrowPrimitiveType,
    {
        let schema = Arc::new(Schema::new(vec![
            Field::new("xunit_id", DataType::FixedSizeBinary(32), true),
            Field::new(column, T::DATA_TYPE, true),
        ]));
        let mut id_builder = FixedSizeBinaryBuilder::new(32);
        for (id, _) in rows {
            id_builder.append_value(id).unwrap();
        }
        let array = PrimitiveArray::<T>::from_iter(rows.iter().map(|(_, v)| *v));
        RecordBatch::try_new(schema, vec![Arc::new(id_builder.finish()), Arc::new(array)]).unwrap()
    }

    #[test]
    fn series_response_scalar_count_sums_only_the_resolved_selectors_rows() {
        // Milestone 14's widening: a count measure folds the Int64 column
        // — and the same selector filter as avg applies first, so a
        // foreign cell's big count must not leak into the sum (the #19
        // shape again, scalar edition).
        let coverage = one_segment_coverage_plan();
        let mobile = XUnit::new(vec![
            YPath::new("device").with_attribute("device", "mobile"),
        ]);
        let batches = vec![vec![scalar_batch::<Int64Type>(
            "views_v1",
            global_id(),
            &[Some(3), Some(4)],
        )]];
        let mixed = vec![vec![scalar_batch_multi::<Int64Type>(
            "views_v1",
            &[
                (global_id(), Some(3)),
                (content_id(&mobile), Some(1000)),
                (global_id(), Some(4)),
            ],
        )]];

        let response = SeriesResponse::new(
            &coverage,
            GapPolicy::Missing,
            AggKind::Count,
            "views_v1",
            &[XUnit::global()],
            &batches,
        )
        .unwrap();
        assert_eq!(response.points[0].value, Some(7.0));

        let response = SeriesResponse::new(
            &coverage,
            GapPolicy::Missing,
            AggKind::Count,
            "views_v1",
            &[XUnit::global()],
            &mixed,
        )
        .unwrap();
        assert_eq!(
            response.points[0].value,
            Some(7.0),
            "scalar fold must also skip the mobile cell's 1000 rows"
        );
    }

    #[test]
    fn series_response_scalar_no_data_is_none_under_missing_and_zero_under_zero_gap_policy() {
        // The scalar path's no-data signal is the seen-flag, not a numeric
        // default — an empty batch list under Missing stays None (min of
        // nothing is not 0), and Zero substitutes exactly there.
        let coverage = one_segment_coverage_plan();
        let batches = vec![vec![scalar_batch::<Float64Type>(
            "latency_v1",
            global_id(),
            &[],
        )]];

        let response = SeriesResponse::new(
            &coverage,
            GapPolicy::Missing,
            AggKind::Min,
            "latency_v1",
            &[XUnit::global()],
            &batches,
        )
        .unwrap();
        assert_eq!(response.points[0].value, None);

        let response = SeriesResponse::new(
            &coverage,
            GapPolicy::Zero,
            AggKind::Min,
            "latency_v1",
            &[XUnit::global()],
            &batches,
        )
        .unwrap();
        assert_eq!(response.points[0].value, Some(0.0));
    }

    #[test]
    fn series_response_rejects_blob_backed_kinds_beyond_avg() {
        // The widening stops at the four scalar kinds; Variance and the
        // other blob-backed kinds still have no decode wiring, so they
        // must be rejected rather than silently mis-read.
        let coverage = one_segment_coverage_plan();
        let batches = vec![vec![avg_batch("var_v1", global_id(), &[])]];
        let err = SeriesResponse::new(
            &coverage,
            GapPolicy::Missing,
            AggKind::Variance,
            "var_v1",
            &[XUnit::global()],
            &batches,
        )
        .expect_err("blob-backed kinds beyond avg must be rejected, not guessed at");
        assert!(matches!(err, CubismError::Temporal(_)));
    }

    #[test]
    fn series_response_propagates_missing_and_published_from_partial_segment() {
        // Same shape as range_query's own
        // coverage_plan_mixed_published_and_missing_within_one_segment: one
        // aligned segment backed by two windows, one published, one not.
        let plan = ResolutionPlan {
            resolution: hour(),
            segments: vec![ResolutionSegment {
                range: TimeRange::new(
                    EventTime::from_unix_micros(0),
                    EventTime::from_unix_micros(3_600_000_000),
                )
                .unwrap(),
                aligned: true,
            }],
        };
        let windows = vec![vec![
            (window("w1"), Some(revision(1))),
            (window("w2"), None),
        ]];
        let coverage = CoveragePlan::new(&plan, false, &windows).unwrap();

        let mut a = AverageState::new();
        a.accumulate(7.0).unwrap();
        let batches = vec![vec![avg_batch("avg_v1", global_id(), &[Some(a.encode())])]];

        let response = SeriesResponse::new(
            &coverage,
            GapPolicy::Missing,
            AggKind::Avg,
            "avg_v1",
            &[XUnit::global()],
            &batches,
        )
        .unwrap();
        let point = &response.points[0];
        assert!(
            !point.is_exact,
            "an unaligned-or-partial segment must not be exact"
        );
        assert_eq!(point.value, Some(7.0));
        assert_eq!(point.published, vec![(window("w1"), revision(1))]);
        assert_eq!(point.missing, vec![window("w2")]);
    }
}
