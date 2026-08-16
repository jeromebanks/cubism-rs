//! `TemporalQuery`: the request shape for Phase 5 range queries
//! (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:697-706`,
//! `docs/TIMESERIES_ROADMAP.md`'s Milestone 8).
//!
//! Pure request-shape logic — no DataFusion execution types, no
//! `cubism-iceberg` I/O. `TemporalQuery::new` validates against a caller-
//! supplied `&TemporalSpec` but does not retain it: Milestone 9's
//! `ResolutionPlan` construction takes both a `TemporalQuery` and the spec
//! separately (`docs/TIMESERIES_ROADMAP.md:610`), so storing the spec here
//! would make that signature redundant.
//!
//! Two validations run at construction time:
//! - `start < end`, delegated to [`TimeRange::new`] rather than
//!   re-implemented, so the invariant has one owner. `TemporalQuery::new`
//!   still takes raw `EventTime` bounds (not a pre-built `TimeRange`) so
//!   this milestone's own rejection test exercises this constructor, not
//!   only `TimeRange`'s.
//! - when `resolution` is `Some`, it must be a member of the cube's
//!   supported set (`spec.base_resolution` or one of `spec.rollups`).
//!   `None` means "auto" — *selecting* an actual resolution from the
//!   spec is Milestone 9's `ResolutionPlan`, not this constructor.
//!
//! What this does **not** validate, deliberately out of scope for this
//! milestone: measure names and `XUnit` selectors are not checked against
//! a `CubeSpec` (no `CubeSpec` is threaded through this module at all —
//! only the cube's `name` is kept, as an identifier); a `Resolution::Calendar`
//! value stored in `TemporalSpec.base_resolution`/`rollups` is accepted by
//! the membership check the same as `Fixed`, because `Calendar` support is
//! `TemporalSpec::validate`'s job (`crates/cubism-core/src/temporal.rs:625-633`),
//! not this one's; and `timezone`/display options from the plan's request
//! shape are omitted entirely this slice — `TemporalSpec::validate` already
//! hard-rejects non-`"UTC"` specs, so a free-form timezone request field
//! would be unimplementable without re-deriving that check, and doing so is
//! not this milestone's scope.
//!
//! [`ResolutionPlan`]: Milestone 9 (`docs/TIMESERIES_ROADMAP.md`'s
//! Milestone 9 entry). Given a [`TemporalQuery`] and a `TemporalSpec`, picks
//! **one** resolution (the query's, or an auto-selected one when `None`) and
//! decomposes `query.range` into 1-3 contiguous, non-overlapping segments at
//! that resolution: an optional partial head, an optional aligned interior
//! (itself possibly spanning many whole buckets, kept as a single segment
//! rather than split per-bucket), and an optional partial tail. Non-overlap
//! and full coverage follow from the segments being built contiguously off
//! one shared cursor, not from any cross-resolution reasoning.
//!
//! What this deliberately does **not** do: decompose across *multiple*
//! resolutions (e.g. serve the interior from a coarser rollup and the edges
//! from the base resolution) — the plan's "resolution choice never overlaps
//! or double-counts" (line 741) reads as inviting that, but it depends on a
//! divisibility/coarseness guarantee between a spec's rollups and base
//! resolution that only `TemporalSpec::validate` enforces
//! (`crates/cubism-core/src/temporal.rs:634-649`) and that this module does
//! not call. Left for a later milestone once that premise is load-bearing
//! (recorded in the roadmap's Milestone 9 entry, not filed as a separate
//! issue — `docs/TIMESERIES_ROADMAP.md` is this series' tracker for
//! Phase-4+ milestone-sized work per #15). It also does not decide `is_exact`,
//! `coverage`, or `source_resolution` — those are Milestone 10's
//! `CoveragePlan` fields, not this one's.
//!
//! A `Resolution::Calendar` value is rejected explicitly wherever this module
//! would otherwise need to do bucket arithmetic on it: no calendar
//! equivalent of `FixedResolution::bucket` exists anywhere in this
//! codebase, and `TemporalQuery::new` deliberately lets a `Calendar` base or
//! rollup pass its membership check (that rejection is `TemporalSpec::validate`'s
//! job, not the constructor's) — so a `Calendar` resolution can legally reach
//! `ResolutionPlan::new` and must be caught here instead of panicking or
//! silently producing a plan that doesn't cover the range.

use cubism_core::{
    BucketOrigin, CubismError, EventTime, FixedResolution, Resolution, TemporalSpec, TimeRange,
    XUnit,
};

/// How a segment with no backing data should be reported. Forwarded to
/// later milestones (`ResolutionPlan`/`CoveragePlan`); not exercised by any
/// logic in this module beyond being a plain field. Named after
/// `cubism_core::BucketValue`'s `Missing` variant (`crates/cubism-core/src/temporal.rs:523`)
/// deliberately: `BucketValue<T>` is the per-bucket *result* type
/// distinguishing an absent bucket from a present zero, and this policy is
/// what a later milestone reads to decide which of the two to construct.
/// Not `BucketValue` itself — that type is generic over a present value
/// `T` this request-shape struct has no reason to carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapPolicy {
    /// Report the segment as `missing` rather than filling a value.
    Missing,
    /// Fill the segment with a zero value.
    Zero,
}

/// A validated request for a temporal range over one cube.
///
/// See the module doc comment for exactly what construction does and does
/// not validate.
#[derive(Debug, Clone, PartialEq)]
pub struct TemporalQuery {
    pub cube: String,
    pub selectors: Vec<XUnit>,
    pub measures: Vec<String>,
    pub range: TimeRange,
    /// `None` means auto-resolution: Milestone 9 picks it from the spec.
    pub resolution: Option<Resolution>,
    pub exact: bool,
    pub gap_policy: GapPolicy,
}

impl TemporalQuery {
    /// A builder was deliberately not introduced for the 9-argument
    /// constructor below — out of scope for a request-shape-only
    /// milestone — so `cargo clippy`'s `too_many_arguments` lint is
    /// suppressed here rather than worked around.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cube: impl Into<String>,
        selectors: Vec<XUnit>,
        measures: Vec<String>,
        start: EventTime,
        end: EventTime,
        resolution: Option<Resolution>,
        exact: bool,
        gap_policy: GapPolicy,
        spec: &TemporalSpec,
    ) -> Result<Self, CubismError> {
        let range = TimeRange::new(start, end)?;
        if let Some(requested) = resolution {
            let supported = requested == spec.base_resolution || spec.rollups.contains(&requested);
            if !supported {
                return Err(CubismError::Temporal(format!(
                    "requested resolution {requested} is not supported by this cube's temporal \
                     spec (base resolution {}, rollups [{}])",
                    spec.base_resolution,
                    spec.rollups
                        .iter()
                        .map(Resolution::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
        Ok(Self {
            cube: cube.into(),
            selectors,
            measures,
            range,
            resolution,
            exact,
            gap_policy,
        })
    }
}

/// One contiguous piece of a [`ResolutionPlan`]'s decomposition.
///
/// `aligned` is true iff both `range.start()` and `range.end()` coincide
/// exactly with resolution-bucket boundaries — true for the (at most one)
/// interior segment, false for a partial head/tail clipped by the query's
/// own `[start, end)` bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolutionSegment {
    pub range: TimeRange,
    pub aligned: bool,
}

/// A single resolution plus the non-overlapping segments that exactly cover
/// a [`TemporalQuery`]'s range at that resolution.
///
/// See the module doc comment for what this does and does not decide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionPlan {
    pub resolution: Resolution,
    pub segments: Vec<ResolutionSegment>,
}

impl ResolutionPlan {
    /// Builds a plan for `query` against `spec`. `spec` should be the same
    /// spec `query` was constructed against — this does not re-run
    /// `TemporalQuery::new`'s membership check.
    pub fn new(query: &TemporalQuery, spec: &TemporalSpec) -> Result<Self, CubismError> {
        let (resolution, fixed) = match query.resolution {
            Some(requested) => (requested, as_fixed(requested)?),
            None => {
                let fixed = auto_select_resolution(spec, query.range)?;
                (Resolution::Fixed(fixed), fixed)
            }
        };
        let segments = decompose(query.range, fixed, spec.origin)?;
        Ok(Self {
            resolution,
            segments,
        })
    }
}

/// Rejects `Calendar`, since no calendar equivalent of `FixedResolution::bucket`
/// exists to decompose a range with. See the module doc comment.
fn as_fixed(resolution: Resolution) -> Result<FixedResolution, CubismError> {
    match resolution {
        Resolution::Fixed(fixed) => Ok(fixed),
        Resolution::Calendar(calendar) => Err(CubismError::Temporal(format!(
            "resolution {calendar} is a calendar resolution; ResolutionPlan only supports \
             fixed-width bucket arithmetic, and no calendar equivalent of \
             FixedResolution::bucket exists"
        ))),
    }
}

/// Picks the coarsest `Fixed` candidate (the spec's base resolution, or one
/// of its rollups) whose width fits within `range`'s duration, falling back
/// to the base resolution when no rollup fits. `Calendar` rollups are not
/// candidates (excluded, not rejected — the spec is not otherwise being
/// validated here); a `Calendar` *base* resolution is rejected via
/// [`as_fixed`], since it leaves no fixed candidate to fall back to. This is
/// Milestone 9's own auto-selection rule, recorded in
/// `docs/TIMESERIES_ROADMAP.md`'s Milestone 9 entry: no other rule is
/// dictated by the plan text.
fn auto_select_resolution(
    spec: &TemporalSpec,
    range: TimeRange,
) -> Result<FixedResolution, CubismError> {
    let duration_micros = range.end().unix_micros() - range.start().unix_micros();
    let mut chosen = as_fixed(spec.base_resolution)?;
    for rollup in &spec.rollups {
        if let Resolution::Fixed(candidate) = rollup
            && candidate.micros() <= duration_micros
            && candidate.micros() > chosen.micros()
        {
            chosen = *candidate;
        }
    }
    Ok(chosen)
}

/// Decomposes `range` into 1-3 contiguous segments at `resolution`: an
/// optional partial head, an optional aligned interior, an optional partial
/// tail. See the module doc comment for why non-overlap/coverage follow
/// from this construction.
fn decompose(
    range: TimeRange,
    resolution: FixedResolution,
    origin: BucketOrigin,
) -> Result<Vec<ResolutionSegment>, CubismError> {
    let start = range.start();
    let end = range.end();
    let start_bucket = resolution.bucket(start, origin)?;
    let end_bucket = resolution.bucket(end, origin)?;
    let start_aligned = start_bucket.start.unix_micros() == start.unix_micros();
    let end_aligned = end_bucket.start.unix_micros() == end.unix_micros();

    let interior_start = if start_aligned {
        start
    } else {
        EventTime::from_unix_micros(start_bucket.end.unix_micros())
    };
    let interior_end = if end_aligned {
        end
    } else {
        EventTime::from_unix_micros(end_bucket.start.unix_micros())
    };

    let mut segments = Vec::with_capacity(3);
    if interior_start > interior_end {
        // The whole range fits inside a single resolution bucket: entirely
        // partial, no aligned interior.
        segments.push(ResolutionSegment {
            range: TimeRange::new(start, end)?,
            aligned: false,
        });
        return Ok(segments);
    }
    if start < interior_start {
        segments.push(ResolutionSegment {
            range: TimeRange::new(start, interior_start)?,
            aligned: false,
        });
    }
    if interior_start < interior_end {
        segments.push(ResolutionSegment {
            range: TimeRange::new(interior_start, interior_end)?,
            aligned: true,
        });
    }
    if interior_end < end {
        segments.push(ResolutionSegment {
            range: TimeRange::new(interior_end, end)?,
            aligned: false,
        });
    }
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cubism_core::{AllowedLateness, CalendarResolution};

    fn spec_with(base: Resolution, rollups: Vec<Resolution>) -> TemporalSpec {
        TemporalSpec {
            event_time: "ts".into(),
            ingestion_time: None,
            base_resolution: base,
            origin: BucketOrigin::default(),
            timezone: "UTC".into(),
            allowed_lateness: AllowedLateness::from_micros(0).unwrap(),
            rollups,
            retention: None,
        }
    }

    fn minute() -> Resolution {
        Resolution::Fixed(FixedResolution::from_micros(60_000_000).unwrap())
    }

    fn hour() -> Resolution {
        Resolution::Fixed(FixedResolution::from_micros(3_600_000_000).unwrap())
    }

    #[test]
    fn valid_construction_with_explicit_supported_resolution() {
        let spec = spec_with(minute(), vec![hour()]);
        let query = TemporalQuery::new(
            "web_events",
            vec![XUnit::global()],
            vec!["count".into()],
            EventTime::from_unix_micros(0),
            EventTime::from_unix_micros(3_600_000_000),
            Some(hour()),
            false,
            GapPolicy::Missing,
            &spec,
        )
        .expect("hour is in the spec's rollups, so this must construct");
        assert_eq!(query.resolution, Some(hour()));
        assert_eq!(query.range.start(), EventTime::from_unix_micros(0));
    }

    #[test]
    fn valid_construction_with_auto_resolution() {
        let spec = spec_with(minute(), vec![hour()]);
        let query = TemporalQuery::new(
            "web_events",
            vec![XUnit::global()],
            vec!["count".into()],
            EventTime::from_unix_micros(0),
            EventTime::from_unix_micros(60_000_000),
            None,
            false,
            GapPolicy::Zero,
            &spec,
        )
        .expect("auto resolution (None) skips the membership check entirely");
        assert_eq!(query.resolution, None);
    }

    #[test]
    fn rejects_start_not_before_end() {
        let spec = spec_with(minute(), vec![]);
        let err = TemporalQuery::new(
            "web_events",
            vec![XUnit::global()],
            vec!["count".into()],
            EventTime::from_unix_micros(60_000_000),
            EventTime::from_unix_micros(60_000_000),
            None,
            false,
            GapPolicy::Missing,
            &spec,
        )
        .expect_err("start == end must be rejected: the range is half-open and non-empty");
        assert!(matches!(err, CubismError::Temporal(_)));
    }

    #[test]
    fn rejects_resolution_not_in_base_or_rollups() {
        let spec = spec_with(minute(), vec![hour()]);
        let day = Resolution::Fixed(FixedResolution::from_micros(86_400_000_000).unwrap());
        let err = TemporalQuery::new(
            "web_events",
            vec![XUnit::global()],
            vec!["count".into()],
            EventTime::from_unix_micros(0),
            EventTime::from_unix_micros(86_400_000_000),
            Some(day),
            false,
            GapPolicy::Missing,
            &spec,
        )
        .expect_err("day is neither the base resolution nor a rollup, so this must be rejected");
        assert!(matches!(err, CubismError::Temporal(_)));
    }

    fn query_with(
        start_micros: i64,
        end_micros: i64,
        resolution: Option<Resolution>,
        spec: &TemporalSpec,
    ) -> TemporalQuery {
        TemporalQuery::new(
            "web_events",
            vec![XUnit::global()],
            vec!["count".into()],
            EventTime::from_unix_micros(start_micros),
            EventTime::from_unix_micros(end_micros),
            resolution,
            false,
            GapPolicy::Missing,
            spec,
        )
        .expect("valid query construction")
    }

    /// Asserts the standard `ResolutionPlan` invariants: segments are
    /// contiguous, in order, and exactly cover `expected_range` with no
    /// overlap or gap.
    fn assert_exact_cover(plan: &ResolutionPlan, expected_range: TimeRange) {
        let first = plan.segments.first().expect("at least one segment");
        let last = plan.segments.last().expect("at least one segment");
        assert_eq!(first.range.start(), expected_range.start());
        assert_eq!(last.range.end(), expected_range.end());
        for pair in plan.segments.windows(2) {
            assert_eq!(
                pair[0].range.end(),
                pair[1].range.start(),
                "segments must be contiguous with no overlap or gap"
            );
        }
    }

    #[test]
    fn resolution_plan_aligned_interval_is_one_segment() {
        let spec = spec_with(minute(), vec![hour()]);
        // [0, 3_600_000_000) at hour resolution (3_600_000_000us) is exactly
        // one aligned hour bucket: no partial head or tail.
        let query = query_with(0, 3_600_000_000, Some(hour()), &spec);
        let plan = ResolutionPlan::new(&query, &spec).expect("aligned hour range");
        assert_eq!(plan.resolution, hour());
        assert_eq!(plan.segments.len(), 1);
        assert!(plan.segments[0].aligned);
        assert_exact_cover(&plan, query.range);
    }

    #[test]
    fn resolution_plan_unaligned_interval_has_head_interior_tail() {
        let spec = spec_with(minute(), vec![hour()]);
        // [600_000_000, 7_800_000_000) at hour resolution: starts 10 minutes
        // into the first hour bucket [0, 3.6B) and ends 10 minutes into the
        // third [7.2B, 10.8B), with the whole second bucket [3.6B, 7.2B) in
        // between — so this must decompose into a partial head, one aligned
        // interior bucket, and a partial tail.
        let query = query_with(600_000_000, 7_800_000_000, Some(hour()), &spec);
        let plan = ResolutionPlan::new(&query, &spec).expect("unaligned hour range");
        assert_eq!(plan.segments.len(), 3);
        assert!(!plan.segments[0].aligned, "head must be partial");
        assert!(plan.segments[1].aligned, "interior must be aligned");
        assert!(!plan.segments[2].aligned, "tail must be partial");
        assert_exact_cover(&plan, query.range);
    }

    #[test]
    fn resolution_plan_range_within_single_bucket_is_one_partial_segment() {
        let spec = spec_with(minute(), vec![hour()]);
        // [600_000_000, 1_200_000_000) is entirely inside the same hour
        // bucket [0, 3_600_000_000): no aligned interior is possible.
        let query = query_with(600_000_000, 1_200_000_000, Some(hour()), &spec);
        let plan = ResolutionPlan::new(&query, &spec).expect("sub-bucket range");
        assert_eq!(plan.segments.len(), 1);
        assert!(!plan.segments[0].aligned);
        assert_exact_cover(&plan, query.range);
    }

    #[test]
    fn resolution_plan_auto_selects_coarsest_resolution_that_fits() {
        let spec = spec_with(minute(), vec![hour()]);
        // A 2-hour range with resolution: None must auto-select the hour
        // rollup (coarsest candidate whose width still fits the duration),
        // not fall back to the minute base resolution.
        let query = query_with(0, 7_200_000_000, None, &spec);
        let plan = ResolutionPlan::new(&query, &spec).expect("auto resolution selects hour");
        assert_eq!(plan.resolution, hour());
        assert_exact_cover(&plan, query.range);
    }

    #[test]
    fn resolution_plan_auto_falls_back_to_base_when_no_rollup_fits() {
        let spec = spec_with(minute(), vec![hour()]);
        // A 30-minute range is narrower than the hour rollup, so auto
        // selection must fall back to the minute base resolution.
        let query = query_with(0, 1_800_000_000, None, &spec);
        let plan = ResolutionPlan::new(&query, &spec).expect("auto resolution falls back");
        assert_eq!(plan.resolution, minute());
        assert_exact_cover(&plan, query.range);
    }

    #[test]
    fn resolution_plan_rejects_calendar_resolution() {
        // `TemporalQuery::new`'s membership check only checks equality/
        // containment, not fixed-vs-calendar — a spec whose base (or a
        // rollup) is `Calendar`, requested explicitly, passes construction.
        // `ResolutionPlan` is where the calendar-arithmetic gap must be
        // caught instead.
        let calendar = Resolution::Calendar(CalendarResolution::CalendarDay);
        let spec = spec_with(calendar, vec![]);
        let query = query_with(0, 3_600_000_000, Some(calendar), &spec);
        let err = ResolutionPlan::new(&query, &spec)
            .expect_err("a Calendar resolution has no bucket arithmetic to decompose with");
        assert!(matches!(err, CubismError::Temporal(_)));
    }

    #[test]
    fn resolution_plan_auto_select_rejects_calendar_base_with_no_fixed_rollup() {
        // resolution: None with a Calendar base and no Fixed rollup leaves
        // auto-selection with no fixed candidate at all.
        let spec = spec_with(
            Resolution::Calendar(CalendarResolution::CalendarDay),
            vec![],
        );
        let query = query_with(0, 3_600_000_000, None, &spec);
        let err = ResolutionPlan::new(&query, &spec)
            .expect_err("auto-selection has no Fixed candidate to fall back to");
        assert!(matches!(err, CubismError::Temporal(_)));
    }
}
