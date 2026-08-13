//! Pure temporal semantics for sparse time-series cubes.
//!
//! Time is an axis around the XUnit lattice, not another YPath. All persisted
//! membership is event-time based, UTC, and half-open. Ingestion time exists
//! only for watermark/lateness decisions.

use crate::error::CubismError;
use chrono::{DateTime, SecondsFormat};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

const MICROS_PER_MILLI: i64 = 1_000;
const MICROS_PER_SECOND: i64 = 1_000_000;
const MICROS_PER_MINUTE: i64 = 60 * MICROS_PER_SECOND;
const MICROS_PER_HOUR: i64 = 60 * MICROS_PER_MINUTE;
const MICROS_PER_DAY: i64 = 24 * MICROS_PER_HOUR;
const MICROS_PER_WEEK: i64 = 7 * MICROS_PER_DAY;

macro_rules! timestamp_type {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(i64);

        impl $name {
            pub const fn from_unix_micros(micros: i64) -> Self {
                Self(micros)
            }

            pub const fn unix_micros(self) -> i64 {
                self.0
            }
        }
    };
}

timestamp_type!(EventTime);
timestamp_type!(IngestionTime);
timestamp_type!(BucketStart);
timestamp_type!(BucketEnd);

/// A validated, non-empty half-open event-time interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeRange {
    start: EventTime,
    end: EventTime,
}

impl<'de> Deserialize<'de> for TimeRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RawTimeRange {
            start: EventTime,
            end: EventTime,
        }

        let raw = RawTimeRange::deserialize(deserializer)?;
        Self::new(raw.start, raw.end).map_err(serde::de::Error::custom)
    }
}

impl TimeRange {
    pub fn new(start: EventTime, end: EventTime) -> Result<Self, CubismError> {
        if start >= end {
            return Err(CubismError::Temporal(format!(
                "time range must be non-empty and half-open: start {} is not before end {}",
                start.unix_micros(),
                end.unix_micros()
            )));
        }
        Ok(Self { start, end })
    }

    pub const fn start(self) -> EventTime {
        self.start
    }

    pub const fn end(self) -> EventTime {
        self.end
    }

    pub fn contains(self, timestamp: EventTime) -> bool {
        self.start <= timestamp && timestamp < self.end
    }
}

/// A positive fixed duration, represented exactly in integer microseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FixedResolution(i64);

impl FixedResolution {
    pub fn from_micros(micros: i64) -> Result<Self, CubismError> {
        if micros <= 0 {
            return Err(CubismError::Temporal(
                "fixed resolution must be greater than zero".into(),
            ));
        }
        Ok(Self(micros))
    }

    pub const fn micros(self) -> i64 {
        self.0
    }

    pub fn bucket(
        self,
        timestamp: EventTime,
        origin: BucketOrigin,
    ) -> Result<TimeBucket, CubismError> {
        let width = i128::from(self.0);
        let delta = i128::from(timestamp.0) - i128::from(origin.0);
        let start = i128::from(origin.0) + delta.div_euclid(width) * width;
        let end = start + width;
        let start = i64::try_from(start)
            .map_err(|_| CubismError::Temporal("bucket start overflows i64 microseconds".into()))?;
        let end = i64::try_from(end)
            .map_err(|_| CubismError::Temporal("bucket end overflows i64 microseconds".into()))?;
        Ok(TimeBucket {
            start: BucketStart(start),
            end: BucketEnd(end),
        })
    }
}

impl fmt::Display for FixedResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (value, suffix) = if self.0 % MICROS_PER_WEEK == 0 {
            (self.0 / MICROS_PER_WEEK, "w")
        } else if self.0 % MICROS_PER_DAY == 0 {
            (self.0 / MICROS_PER_DAY, "d")
        } else if self.0 % MICROS_PER_HOUR == 0 {
            (self.0 / MICROS_PER_HOUR, "h")
        } else if self.0 % MICROS_PER_MINUTE == 0 {
            (self.0 / MICROS_PER_MINUTE, "m")
        } else if self.0 % MICROS_PER_SECOND == 0 {
            (self.0 / MICROS_PER_SECOND, "s")
        } else if self.0 % MICROS_PER_MILLI == 0 {
            (self.0 / MICROS_PER_MILLI, "ms")
        } else {
            (self.0, "us")
        };
        write!(f, "{value}{suffix}")
    }
}

impl FromStr for FixedResolution {
    type Err = CubismError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let split_at = input
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(input.len());
        let (number, suffix) = input.split_at(split_at);
        if number.is_empty() || suffix.is_empty() {
            return Err(CubismError::Temporal(format!(
                "invalid fixed duration '{input}' (expected e.g. 15m, 1h, or 1d)"
            )));
        }
        let value: i64 = number
            .parse()
            .map_err(|_| CubismError::Temporal(format!("invalid duration value '{number}'")))?;
        let multiplier = match suffix {
            "us" | "µs" => 1,
            "ms" => MICROS_PER_MILLI,
            "s" => MICROS_PER_SECOND,
            "m" => MICROS_PER_MINUTE,
            "h" => MICROS_PER_HOUR,
            "d" => MICROS_PER_DAY,
            "w" => MICROS_PER_WEEK,
            _ => {
                return Err(CubismError::Temporal(format!(
                    "unsupported fixed duration unit '{suffix}'"
                )));
            }
        };
        let micros = value
            .checked_mul(multiplier)
            .ok_or_else(|| CubismError::Temporal(format!("duration '{input}' overflows")))?;
        Self::from_micros(micros)
    }
}

impl Serialize for FixedResolution {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for FixedResolution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Explicit origin for fixed bucket arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BucketOrigin(i64);

impl BucketOrigin {
    pub const UNIX_EPOCH: Self = Self(0);

    pub const fn from_unix_micros(micros: i64) -> Self {
        Self(micros)
    }

    pub const fn unix_micros(self) -> i64 {
        self.0
    }
}

impl fmt::Display for BucketOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            return f.write_str("unix");
        }
        let timestamp = DateTime::from_timestamp_micros(self.0).ok_or(fmt::Error)?;
        f.write_str(&timestamp.to_rfc3339_opts(SecondsFormat::Micros, true))
    }
}

impl FromStr for BucketOrigin {
    type Err = CubismError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input == "unix" {
            return Ok(Self::UNIX_EPOCH);
        }
        let timestamp = DateTime::parse_from_rfc3339(input).map_err(|error| {
            CubismError::Temporal(format!("invalid RFC3339 bucket origin '{input}': {error}"))
        })?;
        Ok(Self(timestamp.timestamp_micros()))
    }
}

impl Serialize for BucketOrigin {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for BucketOrigin {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl Default for BucketOrigin {
    fn default() -> Self {
        Self::UNIX_EPOCH
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeBucket {
    pub start: BucketStart,
    pub end: BucketEnd,
}

impl TimeBucket {
    pub fn contains(self, timestamp: EventTime) -> bool {
        self.start.0 <= timestamp.0 && timestamp.0 < self.end.0
    }
}

/// Calendar resolutions are syntax-level values only in Phase 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalendarResolution {
    CalendarDay,
    CalendarWeek,
    CalendarMonth,
    CalendarQuarter,
    CalendarYear,
}

impl fmt::Display for CalendarResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::CalendarDay => "calendar_day",
            Self::CalendarWeek => "calendar_week",
            Self::CalendarMonth => "calendar_month",
            Self::CalendarQuarter => "calendar_quarter",
            Self::CalendarYear => "calendar_year",
        };
        f.write_str(value)
    }
}

impl FromStr for CalendarResolution {
    type Err = CubismError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input {
            "calendar_day" => Ok(Self::CalendarDay),
            "calendar_week" => Ok(Self::CalendarWeek),
            "calendar_month" => Ok(Self::CalendarMonth),
            "calendar_quarter" => Ok(Self::CalendarQuarter),
            "calendar_year" => Ok(Self::CalendarYear),
            _ => Err(CubismError::Temporal(format!(
                "unsupported calendar resolution '{input}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resolution {
    Fixed(FixedResolution),
    Calendar(CalendarResolution),
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixed(resolution) => resolution.fmt(f),
            Self::Calendar(resolution) => resolution.fmt(f),
        }
    }
}

impl FromStr for Resolution {
    type Err = CubismError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.starts_with("calendar_") {
            return Ok(Self::Calendar(input.parse()?));
        }
        Ok(Self::Fixed(input.parse()?))
    }
}

impl Serialize for Resolution {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Resolution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AllowedLateness(i64);

impl AllowedLateness {
    pub fn from_micros(micros: i64) -> Result<Self, CubismError> {
        if micros < 0 {
            return Err(CubismError::Temporal(
                "allowed lateness must not be negative".into(),
            ));
        }
        Ok(Self(micros))
    }

    pub const fn micros(self) -> i64 {
        self.0
    }
}

impl fmt::Display for AllowedLateness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            f.write_str("0s")
        } else {
            FixedResolution(self.0).fmt(f)
        }
    }
}

impl FromStr for AllowedLateness {
    type Err = CubismError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if matches!(input, "0us" | "0ms" | "0s" | "0m" | "0h" | "0d" | "0w") {
            return Ok(Self(0));
        }
        Ok(Self(input.parse::<FixedResolution>()?.micros()))
    }
}

impl Serialize for AllowedLateness {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for AllowedLateness {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Whether an event's timestamp falls inside or past a window's
/// allowed-lateness bound. See [`LatenessPolicy::classify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lateness {
    OnTime,
    Late,
}

/// Classifies an event as on-time or late against a window boundary plus a
/// configured [`AllowedLateness`] bound (`docs/TIMESERIES_ROADMAP.md`
/// Milestone 1 — a building block for the Phase 4 correction machinery in
/// `docs/TIMESERIES_IMPLEMENTATION_PLAN.md` lines 582-670; it does not by
/// itself close any of that plan section's test-list items).
///
/// This is a pure boundary check: [`classify`](Self::classify) takes the
/// window's `bucket_end` as a parameter instead of consulting a
/// publication/control store or an ingestion-time watermark, so a `Late`
/// result says nothing about whether the window has actually been published
/// yet — that reconciliation is `cubism-iceberg`'s `PublicationStore`
/// (Milestone 2), layered on top of this, not this type's job.
///
/// Two-valued by design: this milestone does not add a third "too old,
/// reject as backfill" classification. Plan line 662 ("when a correction is
/// too old and must be rejected or handled as a backfill") stays an open
/// unresolved decision, not silently answered here.
///
/// Resolves plan line 659 ("window duration versus correction blast
/// radius"): the allowed-lateness bound is independent of window duration —
/// `LatenessPolicy` is constructed from an [`AllowedLateness`] alone, never
/// a window's own [`Resolution`], so a caller who wants to bound a
/// correction's blast radius does so by choosing a small `AllowedLateness`
/// directly; a coarser window resolution does not implicitly widen it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LatenessPolicy {
    allowed: AllowedLateness,
}

impl LatenessPolicy {
    pub const fn new(allowed: AllowedLateness) -> Self {
        Self { allowed }
    }

    /// `OnTime` iff `event_time` is strictly before `bucket_end + allowed`
    /// — half-open, matching this module's other boundary types (e.g.
    /// [`TimeBucket::contains`]): the deadline instant itself is `Late`, not
    /// `OnTime`.
    pub fn classify(&self, event_time: EventTime, bucket_end: BucketEnd) -> Lateness {
        let deadline = bucket_end.unix_micros() + self.allowed.micros();
        if event_time.unix_micros() < deadline {
            Lateness::OnTime
        } else {
            Lateness::Late
        }
    }
}

/// Query truthfulness: approximations and incomplete coverage cannot claim
/// exactness merely because a scalar was produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum Exactness {
    Exact,
    Inexact(String),
}

/// Requested and actually represented event-time intervals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub requested: TimeRange,
    #[serde(default)]
    pub covered: Vec<TimeRange>,
    pub exactness: Exactness,
}

/// Distinguishes a missing bucket from a present aggregate whose value is zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum BucketValue<T> {
    Missing,
    Present(T),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct WindowId(String);

impl WindowId {
    pub fn new(value: impl Into<String>) -> Result<Self, CubismError> {
        let value = value.into();
        if value.is_empty() {
            return Err(CubismError::Temporal("window id must not be empty".into()));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for WindowId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct WindowRevision(u64);

impl WindowRevision {
    pub fn new(value: u64) -> Result<Self, CubismError> {
        if value == 0 {
            return Err(CubismError::Temporal(
                "window revision must be greater than zero".into(),
            ));
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for WindowRevision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// `cubism/v2alpha1` temporal configuration. Phase 1 validates only fixed UTC
/// buckets; calendar syntax is retained so it can fail explicitly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemporalSpec {
    pub event_time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingestion_time: Option<String>,
    pub base_resolution: Resolution,
    #[serde(default)]
    pub origin: BucketOrigin,
    #[serde(default = "utc_timezone")]
    pub timezone: String,
    pub allowed_lateness: AllowedLateness,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rollups: Vec<Resolution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<FixedResolution>,
}

fn utc_timezone() -> String {
    "UTC".into()
}

impl TemporalSpec {
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.event_time.trim().is_empty() {
            errors.push("temporal.eventTime must not be empty".into());
        }
        if self
            .ingestion_time
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        {
            errors.push("temporal.ingestionTime must not be empty when present".into());
        }
        if self.timezone != "UTC" {
            errors.push(format!(
                "temporal.timezone '{}' is unsupported in v2alpha1 (expected 'UTC')",
                self.timezone
            ));
        }
        let base = match self.base_resolution {
            Resolution::Fixed(base) => Some(base),
            Resolution::Calendar(value) => {
                errors.push(format!(
                    "calendar resolution '{value}' is parsed but not supported in v2alpha1"
                ));
                None
            }
        };
        for rollup in &self.rollups {
            match (*rollup, base) {
                (Resolution::Calendar(value), _) => errors.push(format!(
                    "calendar rollup '{value}' is parsed but not supported in v2alpha1"
                )),
                (Resolution::Fixed(value), Some(base)) if value.micros() <= base.micros() => {
                    errors.push(format!(
                        "rollup {value} must be coarser than base resolution {base}"
                    ));
                }
                (Resolution::Fixed(value), Some(base)) if value.micros() % base.micros() != 0 => {
                    errors.push(format!(
                        "rollup {value} must be an integer multiple of base resolution {base}"
                    ));
                }
                _ => {}
            }
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_half_open_ranges() {
        let range = TimeRange::new(
            EventTime::from_unix_micros(10),
            EventTime::from_unix_micros(20),
        )
        .unwrap();
        assert!(range.contains(EventTime::from_unix_micros(10)));
        assert!(range.contains(EventTime::from_unix_micros(19)));
        assert!(!range.contains(EventTime::from_unix_micros(20)));
        assert!(
            TimeRange::new(
                EventTime::from_unix_micros(20),
                EventTime::from_unix_micros(20)
            )
            .is_err()
        );
    }

    #[test]
    fn assigns_pre_origin_times_with_euclidean_division() {
        let resolution: FixedResolution = "10s".parse().unwrap();
        let bucket = resolution
            .bucket(EventTime::from_unix_micros(-1), BucketOrigin::UNIX_EPOCH)
            .unwrap();
        assert_eq!(bucket.start.unix_micros(), -10 * MICROS_PER_SECOND);
        assert_eq!(bucket.end.unix_micros(), 0);
        assert!(bucket.contains(EventTime::from_unix_micros(-1)));
        assert!(!bucket.contains(EventTime::from_unix_micros(0)));
    }

    #[test]
    fn duration_parse_round_trip_and_overflow() {
        for value in ["1us", "5ms", "30s", "15m", "6h", "1d", "2w"] {
            let parsed: FixedResolution = value.parse().unwrap();
            assert_eq!(
                parsed.to_string().parse::<FixedResolution>().unwrap(),
                parsed
            );
        }
        assert!("0s".parse::<FixedResolution>().is_err());
        assert!("999999999999999999999d".parse::<FixedResolution>().is_err());
    }

    #[test]
    fn rfc3339_origin_round_trip() {
        let origin: BucketOrigin = "2026-07-28T12:34:56.123456Z".parse().unwrap();
        assert_eq!(origin.to_string().parse::<BucketOrigin>().unwrap(), origin);
    }

    #[test]
    fn calendar_resolutions_are_parsed_but_gated() {
        let spec = TemporalSpec {
            event_time: "occurred_at".into(),
            ingestion_time: None,
            base_resolution: "calendar_month".parse().unwrap(),
            origin: BucketOrigin::UNIX_EPOCH,
            timezone: "UTC".into(),
            allowed_lateness: "1h".parse().unwrap(),
            rollups: Vec::new(),
            retention: None,
        };
        assert!(
            spec.validate()
                .join("\n")
                .contains("parsed but not supported")
        );
    }

    #[test]
    fn missing_is_distinct_from_present_zero() {
        assert_ne!(BucketValue::<u64>::Missing, BucketValue::Present(0));
    }

    #[test]
    fn zero_allowed_lateness_is_valid() {
        let lateness: AllowedLateness = "0s".parse().unwrap();
        assert_eq!(lateness.micros(), 0);
        assert_eq!(lateness.to_string(), "0s");
    }

    #[test]
    fn lateness_policy_classifies_on_the_allowed_lateness_boundary() {
        let policy = LatenessPolicy::new("10s".parse().unwrap());
        let bucket_end = BucketEnd::from_unix_micros(0);

        assert_eq!(
            policy.classify(EventTime::from_unix_micros(10 * MICROS_PER_SECOND - 1), bucket_end),
            Lateness::OnTime
        );
        assert_eq!(
            policy.classify(EventTime::from_unix_micros(10 * MICROS_PER_SECOND), bucket_end),
            Lateness::Late
        );
    }
}
