//! Event time -> affected windows: the first half of
//! [#16](https://github.com/jeromebanks/cubism-rs/issues/16), and plan line
//! 599.
//!
//! # The encoding this module introduces, and why it was deferred until now
//!
//! Mapping a bucket to a [`WindowId`] requires a canonical
//! `(bucket_start, resolution) -> WindowId` encoding.
//! `crates/cubism-datafusion/src/range_query.rs` records that no such
//! encoding existed anywhere in this codebase, and that inventing one was
//! **deliberately rejected** as out of scope for a milestone whose job was
//! something else; `crates/cubism-serve/src/series.rs` consequently makes
//! the *caller* supply `(window_id, bucket_start)` pairs, and
//! `docs/TIMESERIES_ROADMAP.md`'s Milestone 12a assigns ownership of the
//! convention to the demo.
//!
//! That deferral cannot survive #16. "Identify the windows a source
//! correction touches" *is* the encoding question — there is no way to name
//! an affected window without one. So this module defines it, and it is now
//! the canonical answer rather than a private convention.
//!
//! # The encoding
//!
//! A window is named by the UTC instant its bucket starts:
//!
//! - resolutions that divide a day evenly and are a whole number of days,
//!   landing on a midnight — `YYYY-MM-DD`
//! - bucket starts on a whole second — `YYYY-MM-DDTHH:MM:SSZ`
//! - bucket starts with a sub-second remainder —
//!   `YYYY-MM-DDTHH:MM:SS.ffffffZ`
//!
//! The day form is not a new invention: it is exactly what Milestone 12a's
//! `build_temporal_demo.sh` already passes to `--window-id`, so warehouses
//! built before this module resolve through it unchanged. That
//! compatibility is asserted in this module's tests, not assumed —
//! [`day_form_matches_the_existing_demo_convention`].
//!
//! # Why the encoding must be injective, and how the third form arose
//!
//! [`WindowId`] is the *publication key*. Two bucket starts sharing one id
//! is not a cosmetic clash: `CorrectionCoordinator` compare-and-swaps
//! per-`WindowId`, so a collision silently removes the CAS's window
//! scoping, and a correction can revise one window repeatedly while
//! another is never corrected at all.
//!
//! The original encoding had only the first two forms, and was **not**
//! injective. `cubism_core` accepts `us`/`µs`/`ms` resolution units, so on
//! a `500ms` cube the buckets at `0µs` and `500_000µs` both rendered as
//! `1970-01-01T00:00:00Z` — the second-precision format dropped exactly
//! the digits that distinguished them
//! ([#50](https://github.com/jeromebanks/cubism-rs/issues/50)).
//!
//! The sub-second form is emitted **only when the bucket start actually
//! has a sub-second remainder**, which is what makes this a fix rather
//! than a migration: every id existing warehouses hold is day-form or
//! whole-second-form, and both are byte-for-byte unchanged. Widening all
//! instant-form ids to microsecond precision would have been simpler to
//! state and would have rewritten every id in the field.
//!
//! Injectivity holds because each form is lossless over the values it
//! claims: a whole-second bucket start has nothing below the second to
//! lose, and anything that does gets the six digits. The day and instant
//! forms cannot collide — only the instant forms contain a `T`.
//!
//! [`WindowId`]s are opaque keys and are never parsed back into a time
//! anywhere in this workspace (checked), so the added form needs no
//! reader-side support.
//!
//! # What this module does NOT do
//!
//! It performs no I/O and reads no events. It answers "which windows *would*
//! a change in this time range land in", from the spec's own bucket math.
//! Whether those windows were ever published, and what correcting them
//! entails, is [`crate::engine`]'s job.

use chrono::{DateTime, Utc};
use cubism_core::temporal::{
    BucketStart, EventTime, FixedResolution, Resolution, TemporalSpec, TimeRange, WindowId,
};

use crate::CorrectError;

const MICROS_PER_DAY: i64 = 86_400_000_000;
const MICROS_PER_SECOND: i64 = 1_000_000;

/// One window a correction's time range lands in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffectedWindow {
    pub window_id: WindowId,
    /// The window's own half-open bucket bounds — *not* the caller's
    /// requested range clipped to it. A correction rebuilds a whole window
    /// or none of it: a partial rebuild would publish a revision whose
    /// aggregate covers less than the window it claims to be.
    pub range: TimeRange,
}

/// The canonical `bucket_start -> WindowId` encoding (see the module doc).
pub fn window_id_for(
    bucket_start: BucketStart,
    resolution: FixedResolution,
) -> Result<WindowId, CorrectError> {
    let micros = bucket_start.unix_micros();
    let timestamp = DateTime::<Utc>::from_timestamp_micros(micros).ok_or_else(|| {
        CorrectError::WindowNaming(format!("bucket start {micros}µs is out of representable range"))
    })?;
    // A whole number of days, and the bucket actually lands on a midnight:
    // a 1d resolution with a non-midnight origin is still sub-day-aligned
    // in practice, and naming it `YYYY-MM-DD` would collide two windows
    // onto one id.
    let whole_days = resolution.micros() % MICROS_PER_DAY == 0;
    let on_midnight = micros.rem_euclid(MICROS_PER_DAY) == 0;
    // `rem_euclid`, not `%`, so a pre-epoch bucket start is classified by
    // where it sits inside its second rather than by the sign of the
    // remainder.
    let sub_second = micros.rem_euclid(MICROS_PER_SECOND) != 0;
    let formatted = if whole_days && on_midnight {
        timestamp.format("%Y-%m-%d").to_string()
    } else if sub_second {
        timestamp.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
    } else {
        timestamp.format("%Y-%m-%dT%H:%M:%SZ").to_string()
    };
    WindowId::new(formatted).map_err(|e| CorrectError::WindowNaming(e.to_string()))
}

/// Every window the half-open range `[changed.start, changed.end)` touches,
/// in ascending time order.
///
/// A range that starts mid-bucket still yields that whole bucket: an event
/// changing at 03:00 on a daily cube affects the entire day's aggregate.
pub fn affected_windows(
    spec: &TemporalSpec,
    changed: TimeRange,
) -> Result<Vec<AffectedWindow>, CorrectError> {
    let resolution = match spec.base_resolution {
        Resolution::Fixed(resolution) => resolution,
        Resolution::Calendar(value) => {
            return Err(CorrectError::WindowNaming(format!(
                "calendar resolution '{value}' is parsed but not supported; \
                 affected-window identification needs fixed bucket math"
            )));
        }
    };
    let width = resolution.micros();
    let origin = spec.origin;

    let first = resolution
        .bucket(changed.start(), origin)
        .map_err(|e| CorrectError::WindowNaming(e.to_string()))?;

    // Half-open: an end landing exactly on a bucket boundary does NOT pull
    // that bucket in. `[00:00, 00:00+1d)` on a daily cube is one window, not
    // two — the off-by-one that would otherwise republish an untouched
    // neighbour on every correction.
    let last_instant = EventTime::from_unix_micros(
        changed
            .end()
            .unix_micros()
            .checked_sub(1)
            .ok_or_else(|| CorrectError::WindowNaming("range end underflows".into()))?,
    );
    let last = resolution
        .bucket(last_instant, origin)
        .map_err(|e| CorrectError::WindowNaming(e.to_string()))?;

    let span = last
        .start
        .unix_micros()
        .checked_sub(first.start.unix_micros())
        .ok_or_else(|| CorrectError::WindowNaming("window span overflows".into()))?;
    let count = span / width + 1;

    let mut windows = Vec::new();
    for step in 0..count {
        let start = first
            .start
            .unix_micros()
            .checked_add(step.checked_mul(width).ok_or_else(|| {
                CorrectError::WindowNaming("window enumeration overflows".into())
            })?)
            .ok_or_else(|| CorrectError::WindowNaming("window enumeration overflows".into()))?;
        let end = start
            .checked_add(width)
            .ok_or_else(|| CorrectError::WindowNaming("window end overflows".into()))?;
        let bucket_start = BucketStart::from_unix_micros(start);
        windows.push(AffectedWindow {
            window_id: window_id_for(bucket_start, resolution)?,
            range: TimeRange::new(
                EventTime::from_unix_micros(start),
                EventTime::from_unix_micros(end),
            )
            .map_err(|e| CorrectError::WindowNaming(e.to_string()))?,
        });
    }
    Ok(windows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cubism_core::temporal::{AllowedLateness, BucketOrigin};

    fn daily_spec() -> TemporalSpec {
        TemporalSpec {
            event_time: "timestamp".into(),
            ingestion_time: None,
            base_resolution: Resolution::Fixed(FixedResolution::from_micros(MICROS_PER_DAY).unwrap()),
            origin: BucketOrigin::default(),
            timezone: "UTC".into(),
            allowed_lateness: AllowedLateness::from_micros(0).unwrap(),
            rollups: Vec::new(),
            retention: None,
        }
    }

    fn at(text: &str) -> EventTime {
        EventTime::from_unix_micros(
            text.parse::<DateTime<Utc>>().unwrap().timestamp_micros(),
        )
    }

    /// The compatibility constraint this whole encoding is pinned by: the
    /// demo has been writing `--window-id 2026-04-06` since Milestone 12a,
    /// and warehouses hold those ids. A different day format here would
    /// silently fail to find any existing window.
    #[test]
    fn day_form_matches_the_existing_demo_convention() {
        let resolution = FixedResolution::from_micros(MICROS_PER_DAY).unwrap();
        let start = BucketStart::from_unix_micros(at("2026-04-06T00:00:00Z").unix_micros());
        assert_eq!(window_id_for(start, resolution).unwrap().as_str(), "2026-04-06");
    }

    #[test]
    fn sub_day_resolutions_get_a_full_instant_form() {
        let hourly = FixedResolution::from_micros(3_600_000_000).unwrap();
        let start = BucketStart::from_unix_micros(at("2026-04-06T13:00:00Z").unix_micros());
        assert_eq!(
            window_id_for(start, hourly).unwrap().as_str(),
            "2026-04-06T13:00:00Z"
        );
    }

    #[test]
    fn a_change_inside_one_day_affects_exactly_that_day() {
        let windows = affected_windows(
            &daily_spec(),
            TimeRange::new(at("2026-04-06T03:00:00Z"), at("2026-04-06T04:00:00Z")).unwrap(),
        )
        .unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].window_id.as_str(), "2026-04-06");
        // The WHOLE day, not the caller's 03:00-04:00 slice.
        assert_eq!(windows[0].range.start(), at("2026-04-06T00:00:00Z"));
        assert_eq!(windows[0].range.end(), at("2026-04-07T00:00:00Z"));
    }

    /// The off-by-one that would republish an untouched neighbour on every
    /// correction: `[day, day+1d)` is half-open and must be ONE window.
    #[test]
    fn an_end_on_a_bucket_boundary_does_not_pull_in_the_next_window() {
        let windows = affected_windows(
            &daily_spec(),
            TimeRange::new(at("2026-04-06T00:00:00Z"), at("2026-04-07T00:00:00Z")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            windows.iter().map(|w| w.window_id.as_str()).collect::<Vec<_>>(),
            ["2026-04-06"]
        );

        // One microsecond past the boundary does pull the next one in.
        let windows = affected_windows(
            &daily_spec(),
            TimeRange::new(
                at("2026-04-06T00:00:00Z"),
                EventTime::from_unix_micros(at("2026-04-07T00:00:00Z").unix_micros() + 1),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            windows.iter().map(|w| w.window_id.as_str()).collect::<Vec<_>>(),
            ["2026-04-06", "2026-04-07"]
        );
    }

    #[test]
    fn a_multi_day_change_yields_every_day_in_order_with_no_gaps() {
        let windows = affected_windows(
            &daily_spec(),
            TimeRange::new(at("2026-04-06T18:00:00Z"), at("2026-04-09T02:00:00Z")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            windows.iter().map(|w| w.window_id.as_str()).collect::<Vec<_>>(),
            ["2026-04-06", "2026-04-07", "2026-04-08", "2026-04-09"]
        );
        // Contiguous: each window's end is the next one's start.
        for pair in windows.windows(2) {
            assert_eq!(pair[0].range.end(), pair[1].range.start());
        }
    }

    /// #50's regression: `WindowId` is the publication key, so two bucket
    /// starts must never share one. Before the sub-second form existed,
    /// both of these rendered as `1970-01-01T00:00:00Z`.
    #[test]
    fn adjacent_sub_second_buckets_do_not_collide() {
        let half_sec = FixedResolution::from_micros(500_000).unwrap();
        let first = window_id_for(BucketStart::from_unix_micros(0), half_sec).unwrap();
        let second = window_id_for(BucketStart::from_unix_micros(500_000), half_sec).unwrap();
        assert_eq!(first.as_str(), "1970-01-01T00:00:00Z");
        assert_eq!(second.as_str(), "1970-01-01T00:00:00.500000Z");
        assert_ne!(first.as_str(), second.as_str());
    }

    /// The tightest resolution `cubism-core` accepts. One microsecond apart
    /// is the smallest gap the encoding has to keep distinct.
    #[test]
    fn adjacent_microsecond_buckets_do_not_collide() {
        let one_us = FixedResolution::from_micros(1).unwrap();
        let ids: Vec<String> = (0..4)
            .map(|n| window_id_for(BucketStart::from_unix_micros(n), one_us).unwrap().as_str().to_string())
            .collect();
        assert_eq!(
            ids,
            [
                "1970-01-01T00:00:00Z",
                "1970-01-01T00:00:00.000001Z",
                "1970-01-01T00:00:00.000002Z",
                "1970-01-01T00:00:00.000003Z",
            ]
        );
    }

    /// Injectivity is the property that actually matters, so assert it
    /// directly over a dense run of buckets rather than inferring it from
    /// a couple of spot checks.
    #[test]
    fn the_encoding_is_injective_across_a_dense_run_of_buckets() {
        use std::collections::HashSet;
        let one_us = FixedResolution::from_micros(1).unwrap();
        // Straddles a second boundary and the epoch, so both the
        // whole-second/sub-second split and the negative path are covered.
        let ids: HashSet<String> = (-2_000..2_000)
            .map(|n| window_id_for(BucketStart::from_unix_micros(n), one_us).unwrap().as_str().to_string())
            .collect();
        assert_eq!(ids.len(), 4_000, "every distinct bucket start needs a distinct id");
    }

    /// Pre-epoch bucket starts are classified by where they sit inside
    /// their second (`rem_euclid`), not by the sign of a remainder.
    #[test]
    fn pre_epoch_sub_second_buckets_render_correctly() {
        let half_sec = FixedResolution::from_micros(500_000).unwrap();
        assert_eq!(
            window_id_for(BucketStart::from_unix_micros(-500_000), half_sec).unwrap().as_str(),
            "1969-12-31T23:59:59.500000Z"
        );
        assert_eq!(
            window_id_for(BucketStart::from_unix_micros(-1_000_000), half_sec).unwrap().as_str(),
            "1969-12-31T23:59:59Z"
        );
    }

    /// A daily resolution whose origin is not midnight is still sub-day
    /// aligned, so it must NOT take the day form — that was already true
    /// before #50 and must survive the third form's introduction.
    #[test]
    fn a_non_midnight_daily_bucket_keeps_an_instant_form() {
        let daily = FixedResolution::from_micros(MICROS_PER_DAY).unwrap();
        let start = BucketStart::from_unix_micros(at("2026-04-06T06:30:00Z").unix_micros());
        assert_eq!(
            window_id_for(start, daily).unwrap().as_str(),
            "2026-04-06T06:30:00Z"
        );
    }

    /// The whole point of emitting the sub-second form *conditionally*:
    /// ids already sitting in warehouses must not change. Both
    /// pre-existing forms are pinned here alongside #50's new one.
    #[test]
    fn existing_id_forms_are_byte_identical_after_the_sub_second_fix() {
        let daily = FixedResolution::from_micros(MICROS_PER_DAY).unwrap();
        let hourly = FixedResolution::from_micros(3_600_000_000).unwrap();
        assert_eq!(
            window_id_for(
                BucketStart::from_unix_micros(at("2026-04-06T00:00:00Z").unix_micros()),
                daily
            )
            .unwrap()
            .as_str(),
            "2026-04-06"
        );
        assert_eq!(
            window_id_for(
                BucketStart::from_unix_micros(at("2026-04-06T13:00:00Z").unix_micros()),
                hourly
            )
            .unwrap()
            .as_str(),
            "2026-04-06T13:00:00Z"
        );
    }

    #[test]
    fn calendar_resolutions_are_refused_explicitly() {
        let mut spec = daily_spec();
        spec.base_resolution =
            Resolution::Calendar(cubism_core::temporal::CalendarResolution::CalendarMonth);
        let err = affected_windows(
            &spec,
            TimeRange::new(at("2026-04-06T00:00:00Z"), at("2026-04-07T00:00:00Z")).unwrap(),
        )
        .expect_err("calendar resolutions have no fixed bucket math");
        assert!(err.to_string().contains("calendar"));
    }
}
