//! `TemporalQuery`: the request shape for Phase 5 range queries
//! (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:697-706`,
//! `docs/TIMESERIES_ROADMAP.md`'s Milestone 8).
//!
//! Pure request-shape logic — no DataFusion execution types, no
//! `cubism-iceberg` I/O. `TemporalQuery::new` validates against a caller-
//! supplied `&TemporalSpec` but does not retain it: Milestone 9's
//! `ResolutionPlan` construction takes both a `TemporalQuery` and the spec
//! separately (`docs/TIMESERIES_ROADMAP.md:651`), so storing the spec here
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
//! and full coverage follow from the segments being built directly off one
//! shared `interior_start`/`interior_end` boundary pair, not from any
//! cross-resolution reasoning.
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
//!
//! [`CoveragePlan`]: Milestone 10 (`docs/TIMESERIES_ROADMAP.md`'s Milestone
//! 10 entry). Resolves a [`ResolutionPlan`]'s segments against real
//! publication state and decides, per segment, whether it can be answered
//! exactly from retained aggregate state.
//!
//! Two deliberate deviations from the roadmap text, both recorded in the
//! roadmap's Milestone 10 entry, not filed as separate issues (same
//! precedent Milestone 9 set for its own scope cuts):
//!
//! - **The segment→[`WindowId`] mapping is a caller-supplied input, not
//!   something this module derives.** No canonical encoding from a
//!   `TimeRange`/`Resolution` pair to a `WindowId` exists anywhere in this
//!   codebase — `WindowId` is an opaque, caller-assigned string
//!   (`crates/cubism-core/src/temporal.rs:527-529`), and `FixedResolution::bucket`
//!   returns a `TimeBucket`, not a `WindowId`. Inventing that encoding here
//!   would be a durable `cubism-core` API decision smuggled into a Phase-5
//!   milestone, exactly the kind of unenforced-guarantee premise Milestone
//!   9's scope fence already rejected once. `CoveragePlan::new` therefore
//!   takes, per segment, the list of windows the caller has already
//!   determined back that segment's time range, each with its currently
//!   published revision (or `None`). A single segment can legitimately be
//!   backed by more than one window: Milestone 9's aligned interior segment
//!   is kept as one segment even when it spans many resolution buckets.
//! - **`cubism-iceberg` stays a `cubism-datafusion` dev-dependency.** It is
//!   not promoted to a normal dependency (roadmap line 542 floated this as
//!   "one line"): nothing in this module calls `AggregateReader::read_window`
//!   or `PublicationStore::current` directly, so `CoveragePlan` stays pure,
//!   synchronous computation, consistent with `TemporalQuery`/`ResolutionPlan`
//!   above and with this crate not carrying a non-dev `tokio` dependency.
//!   The async iceberg calls that resolve the caller-supplied window list
//!   live in the calling test/service layer instead (this milestone's own
//!   integration test, `crates/cubism-datafusion/tests/iceberg_bridge.rs`,
//!   is exactly that wiring).
//!
//! What this does **not** do: decode or merge any `AggregateState` blob, or
//! produce a value/presentation for a segment — the roadmap's own **Test**
//! bullet for this milestone (`docs/TIMESERIES_ROADMAP.md:689-694`) asserts
//! only provenance, the `missing` marker, and the `exact=true` failure
//! behavior, none of which need a decoded value. The decode+merge primitive
//! itself now exists ([`crate::series_merge::merge_average_column`],
//! Milestone 10b-1), but nothing in this module calls it: wiring a
//! `CoveragePlan`'s `published` list to an actual materialized value is
//! [`crate::series_response::SeriesResponse`]'s job (Milestone 10b-2).
//!
//! A segment is exact iff it is **both** resolution-aligned (`aligned:
//! true`) **and** every window backing it is published. An unaligned
//! (partial head/tail) segment is never exact, even when every window
//! backing it is published: the plan's own words for this
//! (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:721-722`) are "`exact=true`
//! fails clearly if retained buckets/raw data cannot exactly cover a
//! partial boundary" — a published window's aggregate state is a
//! whole-bucket summary, not a sub-bucket one, so it cannot exactly answer
//! a range narrower than the bucket without a raw-event scan, which this
//! milestone does not implement.

use cubism_core::{
    BucketOrigin, CubismError, EventTime, FixedResolution, Resolution, TemporalSpec, TimeRange,
    WindowId, WindowRevision, XUnit,
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
    // i128, not i64: `TimeRange::new` only enforces `start < end`, so a
    // caller-supplied range near `[i64::MIN, i64::MAX)` would overflow a
    // plain i64 subtraction here (panic in debug, silent wraparound in
    // release) — caught by a cross-model phase review
    // (`docs/phase-reviews/TIMESERIES_PHASE_5_REVIEW.md`), not exercised by
    // any test until this fix's own regression test.
    let duration_micros = range.end().unix_micros() as i128 - range.start().unix_micros() as i128;
    let mut chosen = as_fixed(spec.base_resolution)?;
    for rollup in &spec.rollups {
        if let Resolution::Fixed(candidate) = rollup
            && i128::from(candidate.micros()) <= duration_micros
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

/// One [`ResolutionPlan`] segment resolved against real publication state.
///
/// `published`/`missing` partition the windows the caller supplied for this
/// segment (see the module doc comment): `published` carries each window's
/// currently published revision (provenance), `missing` carries the ids of
/// windows with no published revision at all. Both can be non-empty for the
/// same segment — an interior segment spanning several windows can have
/// some published and some not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentCoverage {
    pub segment: ResolutionSegment,
    pub published: Vec<(WindowId, WindowRevision)>,
    pub missing: Vec<WindowId>,
}

impl SegmentCoverage {
    /// True iff this segment is resolution-aligned, every window backing it
    /// is published, and at least one window actually backs it. See the
    /// module doc comment for why an unaligned segment is never exact,
    /// regardless of publication state.
    ///
    /// The `!self.published.is_empty()` conjunct guards a real gap a
    /// cross-model phase review caught (`docs/phase-reviews/TIMESERIES_PHASE_5_REVIEW.md`):
    /// without it, a segment whose caller-supplied window list is entirely
    /// empty (`windows[i] == []` in `CoveragePlan::new`, as opposed to
    /// containing entries with `None` revisions) has both `published` and
    /// `missing` empty, so `missing.is_empty()` alone was vacuously `true`
    /// — an aligned segment backed by *zero* windows was reported exact.
    pub fn is_exact(&self) -> bool {
        self.segment.aligned && self.missing.is_empty() && !self.published.is_empty()
    }
}

/// A [`ResolutionPlan`] resolved against real publication state: per-segment
/// provenance, missing-window markers, and (when the originating query
/// requested `exact: true`) a hard failure instead of a silent rounding.
///
/// See the module doc comment for what this does and does not decide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveragePlan {
    pub resolution: Resolution,
    pub segments: Vec<SegmentCoverage>,
}

impl CoveragePlan {
    /// `windows` must have exactly one entry per `plan.segments`, in the
    /// same order: the list of `(WindowId, Option<WindowRevision>)` the
    /// caller has determined back that segment's time range (`None`
    /// revision means unpublished). See the module doc comment for why this
    /// mapping is a caller-supplied input rather than something derived
    /// here.
    ///
    /// When `exact` is true, fails with `CubismError::Temporal` naming
    /// every segment that cannot be answered exactly (unaligned, missing a
    /// published window, or both) instead of returning a `CoveragePlan`
    /// that silently rounds or omits them — the plan's own `exact=true`
    /// contract (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:721-722`).
    pub fn new(
        plan: &ResolutionPlan,
        exact: bool,
        windows: &[Vec<(WindowId, Option<WindowRevision>)>],
    ) -> Result<Self, CubismError> {
        if windows.len() != plan.segments.len() {
            return Err(CubismError::Temporal(format!(
                "expected one window list per segment ({} segments, {} entries)",
                plan.segments.len(),
                windows.len()
            )));
        }

        let segments: Vec<SegmentCoverage> = plan
            .segments
            .iter()
            .zip(windows)
            .map(|(segment, entries)| {
                let mut published = Vec::new();
                let mut missing = Vec::new();
                for (window_id, revision) in entries {
                    match revision {
                        Some(revision) => published.push((window_id.clone(), *revision)),
                        None => missing.push(window_id.clone()),
                    }
                }
                SegmentCoverage {
                    segment: *segment,
                    published,
                    missing,
                }
            })
            .collect();

        if exact {
            let inexact: Vec<String> = segments
                .iter()
                .filter(|coverage| !coverage.is_exact())
                .map(|coverage| {
                    format!(
                        "[{}, {}) (aligned={}, missing={:?})",
                        coverage.segment.range.start().unix_micros(),
                        coverage.segment.range.end().unix_micros(),
                        coverage.segment.aligned,
                        coverage
                            .missing
                            .iter()
                            .map(WindowId::as_str)
                            .collect::<Vec<_>>()
                    )
                })
                .collect();
            if !inexact.is_empty() {
                return Err(CubismError::Temporal(format!(
                    "query requested exact=true but these segments cannot be answered exactly \
                     from retained aggregate state: {}",
                    inexact.join(", ")
                )));
            }
        }

        Ok(Self {
            resolution: plan.resolution,
            segments,
        })
    }
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

    /// Regression test for a gap a cross-model phase review caught
    /// (`docs/phase-reviews/TIMESERIES_PHASE_5_REVIEW.md`): `TimeRange::new`
    /// only enforces `start < end`, so nothing stops a caller from
    /// constructing a range near `[i64::MIN, i64::MAX)`. Before the fix,
    /// `auto_select_resolution`'s duration computation was a plain `i64`
    /// subtraction that would panic (debug) or silently wrap (release) on
    /// exactly this input. Calls the private `auto_select_resolution`
    /// directly (not through `ResolutionPlan::new`) so this test isolates
    /// the duration-overflow fix from `FixedResolution::bucket`'s own,
    /// already-`i128`-safe overflow handling in `decompose`.
    #[test]
    fn auto_select_resolution_does_not_overflow_on_extreme_range() {
        let spec = spec_with(minute(), vec![hour()]);
        let range = TimeRange::new(
            EventTime::from_unix_micros(i64::MIN),
            EventTime::from_unix_micros(i64::MAX),
        )
        .unwrap();
        let chosen = auto_select_resolution(&spec, range)
            .expect("must not panic or error on an astronomically large duration");
        assert_eq!(
            Resolution::Fixed(chosen),
            hour(),
            "the hour rollup fits comfortably within an i64::MIN..i64::MAX span"
        );
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
    fn resolution_plan_auto_select_rejects_calendar_base() {
        // resolution: None with a Calendar base: auto_select_resolution
        // rejects via as_fixed(spec.base_resolution) before it ever looks
        // at rollups, so the empty rollup list here isn't what triggers the
        // rejection — a Calendar base with a Fixed rollup present would
        // reject identically.
        let spec = spec_with(
            Resolution::Calendar(CalendarResolution::CalendarDay),
            vec![],
        );
        let query = query_with(0, 3_600_000_000, None, &spec);
        let err = ResolutionPlan::new(&query, &spec)
            .expect_err("a Calendar base resolution has no fixed candidate to select");
        assert!(matches!(err, CubismError::Temporal(_)));
    }

    fn window(id: &str) -> WindowId {
        WindowId::new(id).unwrap()
    }

    fn revision(value: u64) -> WindowRevision {
        WindowRevision::new(value).unwrap()
    }

    #[test]
    fn coverage_plan_is_exact_when_segment_aligned_and_all_windows_published() {
        let spec = spec_with(minute(), vec![hour()]);
        // One aligned hour bucket, one window, published: see
        // resolution_plan_aligned_interval_is_one_segment for the same
        // plan shape.
        let query = query_with(0, 3_600_000_000, Some(hour()), &spec);
        let plan = ResolutionPlan::new(&query, &spec).expect("aligned hour range");
        let windows = vec![vec![(window("w1"), Some(revision(1)))]];

        let coverage = CoveragePlan::new(&plan, false, &windows).expect("all windows published");
        assert_eq!(coverage.segments.len(), 1);
        assert!(coverage.segments[0].is_exact());
        assert_eq!(
            coverage.segments[0].published,
            vec![(window("w1"), revision(1))]
        );
        assert!(coverage.segments[0].missing.is_empty());
    }

    #[test]
    fn coverage_plan_reports_missing_window_without_failing_when_exact_not_requested() {
        let spec = spec_with(minute(), vec![hour()]);
        let query = query_with(0, 3_600_000_000, Some(hour()), &spec);
        let plan = ResolutionPlan::new(&query, &spec).expect("aligned hour range");
        let windows = vec![vec![(window("w1"), None)]];

        let coverage = CoveragePlan::new(&plan, false, &windows)
            .expect("exact: false must not fail on a missing window");
        assert!(!coverage.segments[0].is_exact());
        assert!(coverage.segments[0].published.is_empty());
        assert_eq!(coverage.segments[0].missing, vec![window("w1")]);
    }

    #[test]
    fn coverage_plan_exact_true_fails_on_missing_window() {
        let spec = spec_with(minute(), vec![hour()]);
        let query = query_with(0, 3_600_000_000, Some(hour()), &spec);
        let plan = ResolutionPlan::new(&query, &spec).expect("aligned hour range");
        let windows = vec![vec![(window("w1"), None)]];

        let err = CoveragePlan::new(&plan, true, &windows)
            .expect_err("exact: true must fail clearly when a window is unpublished");
        assert!(matches!(err, CubismError::Temporal(_)));
    }

    #[test]
    fn coverage_plan_exact_true_fails_on_unaligned_segment_even_when_published() {
        let spec = spec_with(minute(), vec![hour()]);
        // Same shape as resolution_plan_range_within_single_bucket_is_one_partial_segment:
        // the single segment is unaligned (a partial sub-bucket range), so
        // even a fully published window cannot answer it exactly — the
        // published aggregate is bucket-granularity, not sub-bucket.
        let query = query_with(600_000_000, 1_200_000_000, Some(hour()), &spec);
        let plan = ResolutionPlan::new(&query, &spec).expect("sub-bucket range");
        assert!(
            !plan.segments[0].aligned,
            "precondition: segment must be unaligned"
        );
        let windows = vec![vec![(window("w1"), Some(revision(1)))]];

        let err = CoveragePlan::new(&plan, true, &windows).expect_err(
            "exact: true must fail on an unaligned segment even when its only window is published",
        );
        assert!(matches!(err, CubismError::Temporal(_)));
    }

    #[test]
    fn coverage_plan_mixed_published_and_missing_within_one_segment() {
        let spec = spec_with(minute(), vec![hour()]);
        // A single aligned interior segment can be backed by more than one
        // window (Milestone 9 keeps a multi-bucket interior as one
        // segment) — mirrors the roadmap's own Milestone 10 test scenario
        // (one window published, one not, within a plan spanning both).
        let query = query_with(0, 7_200_000_000, Some(hour()), &spec);
        let plan = ResolutionPlan::new(&query, &spec).expect("two aligned hour buckets");
        assert_eq!(plan.segments.len(), 1);
        assert!(plan.segments[0].aligned);
        let windows = vec![vec![
            (window("w1"), Some(revision(1))),
            (window("w2"), None),
        ]];

        let coverage = CoveragePlan::new(&plan, false, &windows)
            .expect("exact: false must not fail on a partially-missing segment");
        assert!(!coverage.segments[0].is_exact());
        assert_eq!(
            coverage.segments[0].published,
            vec![(window("w1"), revision(1))]
        );
        assert_eq!(coverage.segments[0].missing, vec![window("w2")]);

        let err = CoveragePlan::new(&plan, true, &windows)
            .expect_err("exact: true must fail when any window backing the segment is missing");
        assert!(matches!(err, CubismError::Temporal(_)));
    }

    #[test]
    fn coverage_plan_rejects_windows_length_mismatch() {
        let spec = spec_with(minute(), vec![hour()]);
        let query = query_with(0, 3_600_000_000, Some(hour()), &spec);
        let plan = ResolutionPlan::new(&query, &spec).expect("aligned hour range");

        let err = CoveragePlan::new(&plan, false, &[])
            .expect_err("one segment but zero window-list entries must be rejected");
        assert!(matches!(err, CubismError::Temporal(_)));
    }

    /// Regression test for a gap a cross-model phase review caught
    /// (`docs/phase-reviews/TIMESERIES_PHASE_5_REVIEW.md`): an aligned
    /// segment whose caller-supplied window list is entirely empty (as
    /// opposed to a list containing entries with `None` revisions) must
    /// not be reported exact — `missing.is_empty()` alone is vacuously true
    /// when there are zero entries to iterate, so `is_exact()` needs the
    /// `!published.is_empty()` conjunct this test pins down.
    #[test]
    fn coverage_plan_empty_window_list_is_not_exact() {
        let spec = spec_with(minute(), vec![hour()]);
        let query = query_with(0, 3_600_000_000, Some(hour()), &spec);
        let plan = ResolutionPlan::new(&query, &spec).expect("aligned hour range");
        let windows: Vec<Vec<(WindowId, Option<WindowRevision>)>> = vec![vec![]];

        let coverage = CoveragePlan::new(&plan, false, &windows)
            .expect("exact: false must not fail on a segment backed by zero windows");
        assert!(
            !coverage.segments[0].is_exact(),
            "aligned but backed by zero windows must never be exact, even though missing is also empty"
        );
        assert!(coverage.segments[0].published.is_empty());
        assert!(coverage.segments[0].missing.is_empty());

        let err = CoveragePlan::new(&plan, true, &windows)
            .expect_err("exact: true must fail when a segment has no backing windows at all");
        assert!(matches!(err, CubismError::Temporal(_)));
    }
}
