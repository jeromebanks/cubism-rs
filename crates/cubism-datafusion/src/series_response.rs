//! Milestone 10b-2 (`docs/TIMESERIES_ROADMAP.md`): `SeriesResponse`, the
//! value/presentation wrapper the roadmap's deferred "Milestone 10b" note
//! described — wiring [`crate::series_merge::merge_average_column`]
//! (Milestone 10b-1) to a [`CoveragePlan`]'s (Milestone 10) per-segment
//! coverage so a query actually gets an answered value, not just provenance.
//!
//! Deliberately narrow, per the roadmap split recorded in this milestone's
//! own entry (same precedent Milestone 10b-1 set splitting off of Milestone
//! 10b):
//! - **Only `AverageState`**, via `merge_average_column`. Widening to
//!   `VarianceState`/`QuantileState`/the sketch-backed kinds is left for a
//!   later slice, once `SeriesResponse` needs to carry more than one measure
//!   kind.
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
//! **Gap policy is consumed, not deferred.** `merge_average_column` over a
//! segment with zero published windows (or windows whose only rows are
//! null) returns `AverageState::new()` — a zero-count state whose
//! `present()` is `None`, not `Some(0.0)`, so "no data" and "a real zero"
//! are already distinguished before `gap_policy` ever applies. `gap_policy`
//! only matters for that `None` case: [`GapPolicy::Missing`] (the default a
//! caller should pick when unsure) leaves it `None`; [`GapPolicy::Zero`]
//! substitutes `Some(0.0)`. A segment with a genuine non-zero-count merged
//! value is never touched by this substitution, gap policy or not.

use cubism_core::{AggregateState, CubismError, EventTime, Resolution, WindowId, WindowRevision};
use datafusion::arrow::array::RecordBatch;

use crate::range_query::{CoveragePlan, GapPolicy};
use crate::series_merge::merge_average_column;

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
    /// The segment's merged `AverageState`, presented (`AggregateState::present`,
    /// i.e. the mean). `None` means no data was folded in for this segment —
    /// either no windows were published or `gap_policy` is
    /// [`GapPolicy::Missing`]; see the module doc comment.
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
    pub fn new(
        coverage: &CoveragePlan,
        gap_policy: GapPolicy,
        column: &str,
        batches: &[Vec<RecordBatch>],
    ) -> Result<Self, CubismError> {
        if batches.len() != coverage.segments.len() {
            return Err(CubismError::Temporal(format!(
                "expected one batch list per segment ({} segments, {} entries)",
                coverage.segments.len(),
                batches.len()
            )));
        }

        let points = coverage
            .segments
            .iter()
            .zip(batches)
            .map(|(segment, segment_batches)| {
                let merged = merge_average_column(segment_batches, column)?;
                let value = match (merged.present(), gap_policy) {
                    (None, GapPolicy::Zero) => Some(0.0),
                    (value, _) => value,
                };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::range_query::{ResolutionPlan, ResolutionSegment, TemporalQuery};
    use cubism_core::{
        AllowedLateness, AverageState, BucketOrigin, FixedResolution, TemporalSpec, TimeRange,
        XUnit,
    };
    use datafusion::arrow::array::BinaryBuilder;
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use std::sync::Arc;

    fn window(id: &str) -> WindowId {
        WindowId::new(id).unwrap()
    }

    fn revision(value: u64) -> WindowRevision {
        WindowRevision::new(value).unwrap()
    }

    fn avg_batch(column: &str, blobs: &[Option<Vec<u8>>]) -> RecordBatch {
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

    fn hour() -> Resolution {
        Resolution::Fixed(FixedResolution::from_micros(3_600_000_000).unwrap())
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
        let batches = vec![vec![avg_batch("avg_v1", &[Some(a.encode())])]];

        let response =
            SeriesResponse::new(&coverage, GapPolicy::Missing, "avg_v1", &batches).unwrap();
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
        let batches = vec![vec![avg_batch("avg_v1", &[])]];

        let response =
            SeriesResponse::new(&coverage, GapPolicy::Missing, "avg_v1", &batches).unwrap();
        assert!(
            response.points[0].is_exact,
            "is_exact reflects CoveragePlan's own published/missing lists, not batches content"
        );
        assert_eq!(response.points[0].value, None);
    }

    #[test]
    fn series_response_no_data_is_zero_under_zero_gap_policy() {
        let coverage = one_segment_coverage_plan();
        let batches = vec![vec![avg_batch("avg_v1", &[])]];

        let response = SeriesResponse::new(&coverage, GapPolicy::Zero, "avg_v1", &batches).unwrap();
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
        let batches = vec![vec![avg_batch("avg_v1", &[Some(a.encode())])]];

        let response = SeriesResponse::new(&coverage, GapPolicy::Zero, "avg_v1", &batches).unwrap();
        assert_eq!(response.points[0].value, Some(3.0));
    }

    #[test]
    fn series_response_rejects_batches_length_mismatch() {
        let coverage = one_segment_coverage_plan();
        let err = SeriesResponse::new(&coverage, GapPolicy::Missing, "avg_v1", &[])
            .expect_err("one segment but zero batch-list entries must be rejected");
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
        let batches = vec![vec![avg_batch("avg_v1", &[Some(a.encode())])]];

        let response =
            SeriesResponse::new(&coverage, GapPolicy::Missing, "avg_v1", &batches).unwrap();
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
