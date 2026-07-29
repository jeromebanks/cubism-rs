//! Versioned, mergeable aggregate-state semantics.
//!
//! Presented scalars are never treated as authoritative merge state. Average
//! persists `(sum,count)`, variance persists Welford/Chan `(count,mean,m2)`,
//! and quantiles use a deterministic fixed-width histogram.

use crate::error::CubismError;
use crate::spec::AggKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct StateVersion(u16);

impl StateVersion {
    pub const V1: Self = Self(1);

    pub fn new(value: u16) -> Result<Self, CubismError> {
        if value == 0 {
            return Err(CubismError::AggregateState(
                "state version must be greater than zero".into(),
            ));
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

impl Default for StateVersion {
    fn default() -> Self {
        Self::V1
    }
}

impl<'de> Deserialize<'de> for StateVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(u16::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateExactness {
    Exact,
    Approximate,
}

/// Conservative algebraic declarations. A `false` value means callers must
/// not use that law for repartitioning, retries, rolling windows, or
/// corrections. `numerical_tolerance` makes floating-point caveats explicit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregateCapabilities {
    pub associative: bool,
    pub commutative: bool,
    pub idempotent: bool,
    pub subtractable: bool,
    pub exactness: AggregateExactness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numerical_tolerance: Option<f64>,
}

impl AggregateCapabilities {
    const fn exact(
        associative: bool,
        commutative: bool,
        idempotent: bool,
        subtractable: bool,
    ) -> Self {
        Self {
            associative,
            commutative,
            idempotent,
            subtractable,
            exactness: AggregateExactness::Exact,
            numerical_tolerance: None,
        }
    }

    const fn floating(
        commutative: bool,
        idempotent: bool,
        subtractable: bool,
        tolerance: f64,
    ) -> Self {
        Self {
            associative: false,
            commutative,
            idempotent,
            subtractable,
            exactness: AggregateExactness::Exact,
            numerical_tolerance: Some(tolerance),
        }
    }
}

/// Capabilities for the authoritative state behind each public aggregate.
///
/// Sum/average/variance/centroid are conservative because their current
/// authoritative states contain IEEE-754 additions: they are equivalent
/// across merge trees only within a declared tolerance, not byte-identical.
pub fn capabilities_for(kind: AggKind) -> AggregateCapabilities {
    match kind {
        AggKind::Sum => AggregateCapabilities::floating(true, false, true, 1e-12),
        AggKind::Count => AggregateCapabilities::exact(true, true, false, true),
        AggKind::Min | AggKind::Max => AggregateCapabilities::exact(true, true, true, false),
        AggKind::Avg | AggKind::Variance => {
            AggregateCapabilities::floating(true, false, false, 1e-12)
        }
        AggKind::CountDistinct => AggregateCapabilities {
            associative: true,
            commutative: true,
            idempotent: true,
            subtractable: false,
            exactness: AggregateExactness::Approximate,
            numerical_tolerance: None,
        },
        // Current pruning loses candidates, so merge is not associative once
        // the tracked set overflows. v2alpha1 temporal validation rejects it.
        AggKind::TopK => AggregateCapabilities {
            associative: false,
            commutative: true,
            idempotent: false,
            subtractable: false,
            exactness: AggregateExactness::Approximate,
            numerical_tolerance: None,
        },
        AggKind::Quantile => AggregateCapabilities {
            associative: true,
            commutative: true,
            idempotent: false,
            subtractable: false,
            exactness: AggregateExactness::Approximate,
            numerical_tolerance: None,
        },
        AggKind::Centroid => AggregateCapabilities {
            associative: false,
            commutative: true,
            idempotent: false,
            subtractable: false,
            exactness: AggregateExactness::Exact,
            numerical_tolerance: Some(1e-12),
        },
        AggKind::ReservoirSample => AggregateCapabilities {
            associative: true,
            commutative: true,
            idempotent: true,
            subtractable: false,
            exactness: AggregateExactness::Approximate,
            numerical_tolerance: None,
        },
    }
}

/// Pure state contract; execution engines may monomorphize this trait or wrap
/// it in an enum rather than use trait-object dispatch.
pub trait AggregateState: Sized {
    type Input;
    type Presented;

    fn accumulate(&mut self, input: Self::Input) -> Result<(), CubismError>;
    fn merge(&self, other: &Self) -> Result<Self, CubismError>;
    fn present(&self) -> Self::Presented;
    fn encode(&self) -> Vec<u8>;
    fn decode(bytes: &[u8]) -> Result<Self, CubismError>;
    fn capabilities() -> AggregateCapabilities;
}

const AVG_MAGIC: &[u8; 3] = b"AVG";
const VAR_MAGIC: &[u8; 3] = b"VAR";
const QNT_MAGIC: &[u8; 3] = b"QNT";
const FORMAT_V1: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AverageState {
    sum: f64,
    count: u64,
}

impl Default for AverageState {
    fn default() -> Self {
        Self { sum: 0.0, count: 0 }
    }
}

impl AverageState {
    pub const fn new() -> Self {
        Self { sum: 0.0, count: 0 }
    }

    pub const fn sum(self) -> f64 {
        self.sum
    }

    pub const fn count(self) -> u64 {
        self.count
    }

    pub fn mean(self) -> Option<f64> {
        (self.count != 0).then(|| self.sum / self.count as f64)
    }
}

impl AggregateState for AverageState {
    type Input = f64;
    type Presented = Option<f64>;

    fn accumulate(&mut self, input: f64) -> Result<(), CubismError> {
        if !input.is_finite() {
            return Err(CubismError::AggregateState(
                "average input must be finite".into(),
            ));
        }
        let sum = self.sum + input;
        if !sum.is_finite() {
            return Err(CubismError::AggregateState(
                "average sum overflowed finite f64 state".into(),
            ));
        }
        self.sum = sum;
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| CubismError::AggregateState("average count overflow".into()))?;
        Ok(())
    }

    fn merge(&self, other: &Self) -> Result<Self, CubismError> {
        let sum = self.sum + other.sum;
        if !sum.is_finite() {
            return Err(CubismError::AggregateState(
                "average sum overflowed finite f64 state".into(),
            ));
        }
        Ok(Self {
            sum,
            count: self
                .count
                .checked_add(other.count)
                .ok_or_else(|| CubismError::AggregateState("average count overflow".into()))?,
        })
    }

    fn present(&self) -> Self::Presented {
        self.mean()
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(20);
        out.extend_from_slice(AVG_MAGIC);
        out.push(FORMAT_V1);
        out.extend_from_slice(&self.sum.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self, CubismError> {
        check_header(bytes, AVG_MAGIC, 20, "average")?;
        let sum = f64::from_le_bytes(bytes[4..12].try_into().unwrap());
        let count = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
        if !sum.is_finite() {
            return Err(CubismError::AggregateState(
                "decoded average sum is not finite".into(),
            ));
        }
        Ok(Self { sum, count })
    }

    fn capabilities() -> AggregateCapabilities {
        capabilities_for(AggKind::Avg)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariancePresentation {
    pub count: u64,
    pub mean: f64,
    pub population_variance: f64,
    pub sample_variance: Option<f64>,
}

/// Welford accumulation and Chan parallel merge state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VarianceState {
    count: u64,
    mean: f64,
    m2: f64,
}

impl Default for VarianceState {
    fn default() -> Self {
        Self::new()
    }
}

impl VarianceState {
    pub const fn new() -> Self {
        Self {
            count: 0,
            mean: 0.0,
            m2: 0.0,
        }
    }

    pub const fn count(self) -> u64 {
        self.count
    }

    pub const fn mean(self) -> Option<f64> {
        if self.count == 0 {
            None
        } else {
            Some(self.mean)
        }
    }

    pub fn population_variance(self) -> Option<f64> {
        (self.count != 0).then(|| self.m2 / self.count as f64)
    }

    pub fn sample_variance(self) -> Option<f64> {
        (self.count > 1).then(|| self.m2 / (self.count - 1) as f64)
    }
}

impl AggregateState for VarianceState {
    type Input = f64;
    type Presented = Option<VariancePresentation>;

    fn accumulate(&mut self, input: f64) -> Result<(), CubismError> {
        if !input.is_finite() {
            return Err(CubismError::AggregateState(
                "variance input must be finite".into(),
            ));
        }
        let new_count = self
            .count
            .checked_add(1)
            .ok_or_else(|| CubismError::AggregateState("variance count overflow".into()))?;
        let delta = input - self.mean;
        self.mean += delta / new_count as f64;
        let delta2 = input - self.mean;
        self.m2 += delta * delta2;
        self.count = new_count;
        Ok(())
    }

    fn merge(&self, other: &Self) -> Result<Self, CubismError> {
        if self.count == 0 {
            return Ok(*other);
        }
        if other.count == 0 {
            return Ok(*self);
        }
        let count = self
            .count
            .checked_add(other.count)
            .ok_or_else(|| CubismError::AggregateState("variance count overflow".into()))?;
        let delta = other.mean - self.mean;
        let left_weight = self.count as f64;
        let right_weight = other.count as f64;
        let total_weight = count as f64;
        Ok(Self {
            count,
            mean: self.mean + delta * right_weight / total_weight,
            m2: self.m2 + other.m2 + delta * delta * left_weight * right_weight / total_weight,
        })
    }

    fn present(&self) -> Self::Presented {
        if self.count == 0 {
            None
        } else {
            Some(VariancePresentation {
                count: self.count,
                mean: self.mean,
                population_variance: self.m2 / self.count as f64,
                sample_variance: self.sample_variance(),
            })
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(28);
        out.extend_from_slice(VAR_MAGIC);
        out.push(FORMAT_V1);
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.mean.to_le_bytes());
        out.extend_from_slice(&self.m2.to_le_bytes());
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self, CubismError> {
        check_header(bytes, VAR_MAGIC, 28, "variance")?;
        let state = Self {
            count: u64::from_le_bytes(bytes[4..12].try_into().unwrap()),
            mean: f64::from_le_bytes(bytes[12..20].try_into().unwrap()),
            m2: f64::from_le_bytes(bytes[20..28].try_into().unwrap()),
        };
        if !state.mean.is_finite() || !state.m2.is_finite() || state.m2 < 0.0 {
            return Err(CubismError::AggregateState(
                "decoded variance state is invalid".into(),
            ));
        }
        if state.count == 0 && (state.mean != 0.0 || state.m2 != 0.0) {
            return Err(CubismError::AggregateState(
                "empty variance state must have zero mean and m2".into(),
            ));
        }
        Ok(state)
    }

    fn capabilities() -> AggregateCapabilities {
        capabilities_for(AggKind::Variance)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuantileState {
    bin_width: f64,
    count: u64,
    bins: BTreeMap<i64, u64>,
}

impl QuantileState {
    pub fn new(bin_width: f64) -> Result<Self, CubismError> {
        if !bin_width.is_finite() || bin_width <= 0.0 {
            return Err(CubismError::AggregateState(
                "quantile bin width must be finite and greater than zero".into(),
            ));
        }
        Ok(Self {
            bin_width,
            count: 0,
            bins: BTreeMap::new(),
        })
    }

    pub const fn count(&self) -> u64 {
        self.count
    }

    pub const fn bin_width(&self) -> f64 {
        self.bin_width
    }

    /// Returns the midpoint of the selected bin. Absolute error from
    /// discretization is at most `bin_width / 2`.
    pub fn quantile(&self, probability: f64) -> Result<Option<f64>, CubismError> {
        if !(0.0..=1.0).contains(&probability) || !probability.is_finite() {
            return Err(CubismError::AggregateState(
                "quantile probability must be finite and between 0 and 1".into(),
            ));
        }
        if self.count == 0 {
            return Ok(None);
        }
        let rank = ((self.count - 1) as f64 * probability).floor() as u64;
        let mut seen = 0_u64;
        for (index, count) in &self.bins {
            seen += count;
            if seen > rank {
                return Ok(Some((*index as f64 + 0.5) * self.bin_width));
            }
        }
        Err(CubismError::AggregateState(
            "quantile state count does not match bins".into(),
        ))
    }

    fn index_for(&self, value: f64) -> Result<i64, CubismError> {
        if !value.is_finite() {
            return Err(CubismError::AggregateState(
                "quantile input must be finite".into(),
            ));
        }
        let index = (value / self.bin_width).floor();
        if index < i64::MIN as f64 || index > i64::MAX as f64 {
            return Err(CubismError::AggregateState(
                "quantile input is outside representable bin range".into(),
            ));
        }
        Ok(index as i64)
    }
}

impl AggregateState for QuantileState {
    type Input = f64;
    type Presented = Option<f64>;

    fn accumulate(&mut self, input: f64) -> Result<(), CubismError> {
        let index = self.index_for(input)?;
        let bin = self.bins.entry(index).or_default();
        *bin = bin
            .checked_add(1)
            .ok_or_else(|| CubismError::AggregateState("quantile bin count overflow".into()))?;
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| CubismError::AggregateState("quantile count overflow".into()))?;
        Ok(())
    }

    fn merge(&self, other: &Self) -> Result<Self, CubismError> {
        if self.bin_width.to_bits() != other.bin_width.to_bits() {
            return Err(CubismError::AggregateState(format!(
                "quantile bin width mismatch: {} versus {}",
                self.bin_width, other.bin_width
            )));
        }
        let mut merged = self.clone();
        merged.count = self
            .count
            .checked_add(other.count)
            .ok_or_else(|| CubismError::AggregateState("quantile count overflow".into()))?;
        for (index, count) in &other.bins {
            let bin = merged.bins.entry(*index).or_default();
            *bin = bin
                .checked_add(*count)
                .ok_or_else(|| CubismError::AggregateState("quantile bin count overflow".into()))?;
        }
        Ok(merged)
    }

    fn present(&self) -> Self::Presented {
        self.quantile(0.5).expect("stored quantile state is valid")
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(24 + self.bins.len() * 16);
        out.extend_from_slice(QNT_MAGIC);
        out.push(FORMAT_V1);
        out.extend_from_slice(&self.bin_width.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&(self.bins.len() as u32).to_le_bytes());
        for (index, count) in &self.bins {
            out.extend_from_slice(&index.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
        }
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self, CubismError> {
        if bytes.len() < 24 || &bytes[..3] != QNT_MAGIC || bytes[3] != FORMAT_V1 {
            return Err(CubismError::AggregateState(
                "quantile has bad magic, version, or a truncated header".into(),
            ));
        }
        let bin_width = f64::from_le_bytes(bytes[4..12].try_into().unwrap());
        let count = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
        let bin_count = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
        let expected = 24_usize
            .checked_add(bin_count.checked_mul(16).ok_or_else(|| {
                CubismError::AggregateState("quantile payload length overflows".into())
            })?)
            .ok_or_else(|| {
                CubismError::AggregateState("quantile payload length overflows".into())
            })?;
        if bytes.len() != expected {
            return Err(CubismError::AggregateState(format!(
                "quantile payload has {} bytes, expected {expected}",
                bytes.len()
            )));
        }
        let mut state = Self::new(bin_width)?;
        state.count = count;
        let mut decoded_count = 0_u64;
        let mut previous = None;
        for chunk in bytes[24..].chunks_exact(16) {
            let index = i64::from_le_bytes(chunk[..8].try_into().unwrap());
            let bin_count = u64::from_le_bytes(chunk[8..].try_into().unwrap());
            if bin_count == 0 || previous.is_some_and(|value| index <= value) {
                return Err(CubismError::AggregateState(
                    "quantile bins must be strictly ordered with positive counts".into(),
                ));
            }
            decoded_count = decoded_count.checked_add(bin_count).ok_or_else(|| {
                CubismError::AggregateState("quantile decoded count overflow".into())
            })?;
            state.bins.insert(index, bin_count);
            previous = Some(index);
        }
        if decoded_count != count {
            return Err(CubismError::AggregateState(format!(
                "quantile count {count} does not match decoded bin count {decoded_count}"
            )));
        }
        Ok(state)
    }

    fn capabilities() -> AggregateCapabilities {
        capabilities_for(AggKind::Quantile)
    }
}

fn check_header(
    bytes: &[u8],
    magic: &[u8; 3],
    length: usize,
    kind: &str,
) -> Result<(), CubismError> {
    if bytes.len() != length || &bytes[..bytes.len().min(3)] != magic {
        return Err(CubismError::AggregateState(format!(
            "{kind} has bad magic or length"
        )));
    }
    if bytes[3] != FORMAT_V1 {
        return Err(CubismError::AggregateState(format!(
            "{kind} uses unsupported state version {}",
            bytes[3]
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn average_persists_sum_and_count() {
        let mut state = AverageState::new();
        for value in [1.0, 2.0, 6.0] {
            state.accumulate(value).unwrap();
        }
        assert_eq!(state.sum(), 9.0);
        assert_eq!(state.count(), 3);
        assert_eq!(state.present(), Some(3.0));
        assert_eq!(AverageState::decode(&state.encode()).unwrap(), state);
    }

    #[test]
    fn variance_merge_matches_single_pass_with_tolerance() {
        let mut left = VarianceState::new();
        let mut right = VarianceState::new();
        let mut direct = VarianceState::new();
        for value in [1.0, 2.0, 3.0] {
            left.accumulate(value).unwrap();
            direct.accumulate(value).unwrap();
        }
        for value in [7.0, 9.0, 11.0] {
            right.accumulate(value).unwrap();
            direct.accumulate(value).unwrap();
        }
        let merged = left.merge(&right).unwrap();
        assert!((merged.mean().unwrap() - direct.mean().unwrap()).abs() < 1e-12);
        assert!(
            (merged.population_variance().unwrap() - direct.population_variance().unwrap()).abs()
                < 1e-12
        );
        assert_eq!(VarianceState::decode(&merged.encode()).unwrap(), merged);
    }

    #[test]
    fn fixed_histogram_quantiles_are_lawfully_mergeable() {
        let mut left = QuantileState::new(1.0).unwrap();
        let mut right = QuantileState::new(1.0).unwrap();
        let mut direct = QuantileState::new(1.0).unwrap();
        for value in [0.1, 1.1, 2.1] {
            left.accumulate(value).unwrap();
            direct.accumulate(value).unwrap();
        }
        for value in [3.1, 4.1, 5.1] {
            right.accumulate(value).unwrap();
            direct.accumulate(value).unwrap();
        }
        let merged = left.merge(&right).unwrap();
        assert_eq!(merged, direct);
        assert_eq!(merged.quantile(0.5).unwrap(), Some(2.5));
        assert_eq!(QuantileState::decode(&merged.encode()).unwrap(), merged);
    }

    #[test]
    fn capability_declarations_are_truthful_for_risky_states() {
        assert!(!capabilities_for(AggKind::TopK).associative);
        assert!(!capabilities_for(AggKind::Centroid).associative);
        assert!(
            capabilities_for(AggKind::Centroid)
                .numerical_tolerance
                .is_some()
        );
        assert!(capabilities_for(AggKind::CountDistinct).idempotent);
    }

    #[test]
    fn golden_state_bytes_v1() {
        let mut average = AverageState::new();
        average.accumulate(1.5).unwrap();
        let average_hex: String = average
            .encode()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(average_hex, "41564701000000000000f83f0100000000000000");

        let mut variance = VarianceState::new();
        variance.accumulate(2.0).unwrap();
        let variance_hex: String = variance
            .encode()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(
            variance_hex,
            "56415201010000000000000000000000000000400000000000000000"
        );
    }
}
