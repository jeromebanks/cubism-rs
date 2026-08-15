//! `TemporalQuery`: the request shape for Phase 5 range queries
//! (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:697-706`,
//! `docs/TIMESERIES_ROADMAP.md`'s Milestone 8).
//!
//! Pure request-shape logic — no DataFusion execution types, no
//! `cubism-iceberg` I/O. `TemporalQuery::new` validates against a caller-
//! supplied `&TemporalSpec` but does not retain it: Milestone 9's
//! `ResolutionPlan` construction takes both a `TemporalQuery` and the spec
//! separately (`docs/TIMESERIES_ROADMAP.md:591`), so storing the spec here
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

use cubism_core::{CubismError, EventTime, Resolution, TemporalSpec, TimeRange, XUnit};

/// How a segment with no backing data should be reported. Forwarded to
/// later milestones (`ResolutionPlan`/`CoveragePlan`); not exercised by any
/// logic in this module beyond being a plain field.
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
            let supported =
                requested == spec.base_resolution || spec.rollups.contains(&requested);
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

#[cfg(test)]
mod tests {
    use super::*;
    use cubism_core::{AllowedLateness, BucketOrigin, FixedResolution};

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
}
