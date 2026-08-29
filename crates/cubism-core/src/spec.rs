//! The declarative cube spec (`apiVersion: v1`) — the public contract.
//!
//! This schema is what OSS users write in YAML and what the future SaaS
//! accepts over its API, so parse and validation errors must be readable by
//! someone who has never seen the Rust types.
//!
//! ```yaml
//! apiVersion: v1
//! name: web_events
//! dimensions:
//!   - name: geo
//!     levels: [country, city]          # shorthand: level name == source column
//!   - name: platform
//!     levels:
//!       - name: device
//!         expr: device_type            # SQL expression evaluated by the engine
//!       - browser
//!   - name: gender
//! filterRules:
//!   - type: max_dimensions
//!     n: 3
//!   - type: not_together
//!     dims: [geo, platform]
//! measures:
//!   - name: pageviews
//!     agg: sum
//!     input: pv
//!   - name: reach
//!     agg: count_distinct
//!     input: user_id
//! includeGlobal: true
//! ```

use crate::aggregate_state::StateVersion;
use crate::error::CubismError;
use crate::rules::FilterRule;
use crate::temporal::TemporalSpec;
use crate::ypath::YPath;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const API_VERSION_V1: &str = "v1";
pub const API_VERSION_V2_ALPHA1: &str = "cubism/v2alpha1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CubeSpec {
    pub api_version: String,
    pub name: String,
    pub dimensions: Vec<DimensionSpec>,
    #[serde(default)]
    pub filter_rules: Vec<FilterRule>,
    #[serde(default)]
    pub measures: Vec<MeasureSpec>,
    #[serde(default)]
    pub include_global: bool,
    /// Present only for explicitly temporal `cubism/v2alpha1` specs. A
    /// dimension named `time` is never inferred or reinterpreted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporal: Option<TemporalSpec>,
    /// Guardrail against high-cardinality dimensions (UUIDs, raw
    /// timestamps): the build fails fast once its string dictionary holds
    /// more than this many distinct interned strings, instead of letting
    /// the cell count explode. See `docs/high-cardinality.md`.
    #[serde(default = "default_max_dictionary_entries")]
    pub max_dictionary_entries: usize,
}

fn default_max_dictionary_entries() -> usize {
    1_000_000
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DimensionSpec {
    pub name: String,
    /// Hierarchy levels, outermost first (`[country, city]`). Omitted levels
    /// default to a single level named after the dimension, so
    /// `- name: gender` is a complete flat dimension.
    #[serde(default)]
    pub levels: Vec<LevelSpec>,
}

/// One hierarchy level: an attribute name plus the SQL expression producing
/// its value. `expr` defaults to the level name (i.e. a plain column
/// reference), and a bare string in YAML is shorthand for that default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "LevelSpecDe", deny_unknown_fields)]
pub struct LevelSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LevelSpecDe {
    Name(String),
    Full {
        name: String,
        #[serde(default)]
        expr: Option<String>,
    },
}

impl From<LevelSpecDe> for LevelSpec {
    fn from(de: LevelSpecDe) -> Self {
        match de {
            LevelSpecDe::Name(name) => LevelSpec { name, expr: None },
            LevelSpecDe::Full { name, expr } => LevelSpec { name, expr },
        }
    }
}

impl LevelSpec {
    /// The SQL expression producing this level's value.
    pub fn expression(&self) -> &str {
        self.expr.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureSpec {
    pub name: String,
    pub agg: AggKind,
    /// Input column/expression. Optional only for `count`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// Score column/expression — required by `top_k` (`input` is the key,
    /// `by` is what it's ranked by), invalid elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// Versioned authoritative state parameters. Defaults are pinned by the
    /// API version and included in the spec hash through this structure.
    #[serde(default, skip_serializing_if = "AggregateStateConfig::is_default")]
    pub state: AggregateStateConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AggregateStateConfig {
    #[serde(default = "default_state_version")]
    pub version: StateVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kmv_size: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantile_bin_width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_capacity: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub centroid_tolerance: Option<f64>,
}

const fn default_state_version() -> StateVersion {
    StateVersion::V1
}

impl Default for AggregateStateConfig {
    fn default() -> Self {
        Self {
            version: default_state_version(),
            kmv_size: None,
            quantile_bin_width: None,
            sample_capacity: None,
            centroid_tolerance: None,
        }
    }
}

impl AggregateStateConfig {
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

/// The v0.1 aggregator set. Sketch-backed kinds carry mergeable buffers
/// (implemented in M3); listed here so specs validate against the real menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggKind {
    Sum,
    Count,
    Min,
    Max,
    Avg,
    /// Mergeable Welford/Chan `(count, mean, m2)` state.
    Variance,
    /// KMV sketch: cardinality + set ops (union/intersection/Jaccard).
    CountDistinct,
    /// Bounded top-N by score (ArgMaxMap).
    TopK,
    /// Mergeable quantile sketch (latency/cost percentiles).
    Quantile,
    /// Dense embedding mean (vector sum + count).
    Centroid,
    /// Bounded uniform sample of exemplar values.
    ReservoirSample,
}

impl AggKind {
    pub fn requires_input(&self) -> bool {
        !matches!(self, AggKind::Count)
    }
}

impl DimensionSpec {
    /// Effective levels: declared levels, or the single implicit level named
    /// after the dimension.
    pub fn effective_levels(&self) -> Vec<LevelSpec> {
        if self.levels.is_empty() {
            vec![LevelSpec {
                name: self.name.clone(),
                expr: None,
            }]
        } else {
            self.levels.clone()
        }
    }

    /// Build the hierarchy-prefix YPaths for one row's level values
    /// (`[Some("CZ"), Some("Prague")]` → `[/geo/country=CZ,
    /// /geo/country=CZ/city=Prague]`). The hierarchy truncates at the first
    /// missing level: a row with country but no city still lands in the
    /// country cell. (The legacy library emitted a literal `null` value
    /// there; truncation replaces that.)
    pub fn ypaths_for_values(&self, values: &[Option<String>]) -> Vec<YPath> {
        let levels = self.effective_levels();
        let mut out = Vec::new();
        let mut yp = YPath::new(self.name.clone());
        for (level, value) in levels.iter().zip(values) {
            match value {
                Some(v) => {
                    yp = yp.with_attribute(level.name.clone(), v.clone());
                    out.push(yp.clone());
                }
                None => break,
            }
        }
        out
    }
}

impl CubeSpec {
    pub fn from_yaml(yaml: &str) -> Result<Self, CubismError> {
        let spec: CubeSpec =
            serde_norway::from_str(yaml).map_err(|e| CubismError::SpecParse(e.to_string()))?;
        spec.validate()?;
        Ok(spec)
    }

    pub fn from_json(json: &str) -> Result<Self, CubismError> {
        let spec: CubeSpec =
            serde_json::from_str(json).map_err(|e| CubismError::SpecParse(e.to_string()))?;
        spec.validate()?;
        Ok(spec)
    }

    pub fn to_yaml(&self) -> String {
        serde_norway::to_string(self).expect("spec serialization is infallible")
    }

    /// Stable 256-bit hash of the validated semantic spec.
    pub fn spec_hash(&self) -> Result<[u8; 32], CubismError> {
        self.validate()?;
        let canonical =
            serde_json::to_vec(self).map_err(|error| CubismError::SpecParse(error.to_string()))?;
        Ok(*blake3::hash(&canonical).as_bytes())
    }

    /// Dimensions in canonical (name-sorted) order — the order lattice
    /// generation folds over, matching legacy `XUnitDefinition`.
    pub fn sorted_dimensions(&self) -> Vec<&DimensionSpec> {
        let mut dims: Vec<&DimensionSpec> = self.dimensions.iter().collect();
        dims.sort_by(|a, b| a.name.cmp(&b.name));
        dims
    }

    /// Validate the whole spec, reporting every problem at once.
    pub fn validate(&self) -> Result<(), CubismError> {
        let mut errors: Vec<String> = Vec::new();

        if self.api_version != API_VERSION_V1 && self.api_version != API_VERSION_V2_ALPHA1 {
            errors.push(format!(
                "apiVersion '{}' is not supported (expected '{API_VERSION_V1}' or \
                 '{API_VERSION_V2_ALPHA1}')",
                self.api_version
            ));
        }

        match (self.api_version.as_str(), &self.temporal) {
            (API_VERSION_V1, Some(_)) => {
                errors.push("apiVersion 'v1' does not support a temporal section".into());
            }
            (API_VERSION_V2_ALPHA1, None) => errors.push(format!(
                "apiVersion '{API_VERSION_V2_ALPHA1}' requires a temporal section"
            )),
            (API_VERSION_V2_ALPHA1, Some(temporal)) => {
                errors.extend(temporal.validate());
            }
            _ => {}
        }

        if self.max_dictionary_entries == 0 {
            errors.push("maxDictionaryEntries must be at least 1".to_string());
        }
        if self.name.is_empty() {
            errors.push("cube 'name' must not be empty".into());
        }
        if self.dimensions.is_empty() {
            errors.push("at least one dimension is required".into());
        }

        let mut dim_names: HashSet<&str> = HashSet::new();
        for dim in &self.dimensions {
            if !dim_names.insert(&dim.name) {
                errors.push(format!("duplicate dimension name '{}'", dim.name));
            }
            if let Some(bad) = name_violation(&dim.name) {
                errors.push(format!("dimension name '{}' {bad}", dim.name));
            }
            let mut level_names: HashSet<&str> = HashSet::new();
            for level in &dim.levels {
                if !level_names.insert(&level.name) {
                    errors.push(format!(
                        "dimension '{}': duplicate level name '{}'",
                        dim.name, level.name
                    ));
                }
                if let Some(bad) = name_violation(&level.name) {
                    errors.push(format!(
                        "dimension '{}': level name '{}' {bad}",
                        dim.name, level.name
                    ));
                }
            }
        }

        for rule in &self.filter_rules {
            for dim in rule.referenced_dims() {
                if !dim_names.contains(dim) {
                    errors.push(format!(
                        "filter rule references unknown dimension '{dim}' \
                         (declared dimensions: {})",
                        joined(&dim_names)
                    ));
                }
            }
            if let FilterRule::MaxDimensions { n: 0 } = rule {
                errors.push("max_dimensions n must be at least 1 (0 excludes every cell)".into());
            }
        }

        let mut measure_names: HashSet<&str> = HashSet::new();
        for measure in &self.measures {
            if !measure_names.insert(&measure.name) {
                errors.push(format!("duplicate measure name '{}'", measure.name));
            }
            if measure.agg.requires_input() && measure.input.is_none() {
                errors.push(format!(
                    "measure '{}': agg '{:?}' requires an 'input' column or expression",
                    measure.name, measure.agg
                ));
            }
            match (measure.agg, &measure.by) {
                (AggKind::TopK, None) => errors.push(format!(
                    "measure '{}': top_k requires 'by' (the score expression; 'input' is the key)",
                    measure.name
                )),
                (AggKind::TopK, Some(_)) => {}
                (_, Some(_)) => errors.push(format!(
                    "measure '{}': 'by' is only valid for top_k",
                    measure.name
                )),
                _ => {}
            }
            validate_state_config(
                measure,
                self.api_version == API_VERSION_V2_ALPHA1,
                &mut errors,
            );
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(CubismError::Validation(errors))
        }
    }
}

fn validate_state_config(measure: &MeasureSpec, temporal: bool, errors: &mut Vec<String>) {
    let state = &measure.state;
    if !temporal && !state.is_default() {
        errors.push(format!(
            "measure '{}': explicit state parameters require apiVersion '{}'",
            measure.name, API_VERSION_V2_ALPHA1
        ));
        return;
    }
    if state.version != StateVersion::V1 {
        errors.push(format!(
            "measure '{}': unsupported aggregate state version {} (expected 1)",
            measure.name,
            state.version.get()
        ));
    }
    if state.kmv_size.is_some() && measure.agg != AggKind::CountDistinct {
        errors.push(format!(
            "measure '{}': state.kmvSize is only valid for count_distinct",
            measure.name
        ));
    }
    if state.quantile_bin_width.is_some() && measure.agg != AggKind::Quantile {
        errors.push(format!(
            "measure '{}': state.quantileBinWidth is only valid for quantile",
            measure.name
        ));
    }
    if state.sample_capacity.is_some() && measure.agg != AggKind::ReservoirSample {
        errors.push(format!(
            "measure '{}': state.sampleCapacity is only valid for reservoir_sample",
            measure.name
        ));
    }
    if state.centroid_tolerance.is_some() && measure.agg != AggKind::Centroid {
        errors.push(format!(
            "measure '{}': state.centroidTolerance is only valid for centroid",
            measure.name
        ));
    }
    if state.kmv_size.is_some_and(|value| value < 2) {
        errors.push(format!(
            "measure '{}': state.kmvSize must be at least 2",
            measure.name
        ));
    }
    if state.sample_capacity == Some(0) {
        errors.push(format!(
            "measure '{}': state.sampleCapacity must be at least 1",
            measure.name
        ));
    }
    if state
        .quantile_bin_width
        .is_some_and(|value| !value.is_finite() || value <= 0.0)
    {
        errors.push(format!(
            "measure '{}': state.quantileBinWidth must be finite and greater than zero",
            measure.name
        ));
    }
    if state
        .centroid_tolerance
        .is_some_and(|value| !value.is_finite() || value <= 0.0)
    {
        errors.push(format!(
            "measure '{}': state.centroidTolerance must be finite and greater than zero",
            measure.name
        ));
    }
    if temporal && measure.agg == AggKind::Quantile && state.quantile_bin_width.is_none() {
        errors.push(format!(
            "measure '{}': temporal quantile requires state.quantileBinWidth",
            measure.name
        ));
    }
    if temporal && measure.agg == AggKind::TopK {
        errors.push(format!(
            "measure '{}': top_k is not supported for temporal aggregation because its \
             current pruned merge is not associative",
            measure.name
        ));
    }
}

/// Dimension and level names appear inside the YPath string format, so the
/// structural characters are forbidden in them (values are escaped; names
/// are not).
fn name_violation(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("must not be empty")
    } else if name.contains(['/', '=', ',']) {
        Some("must not contain '/', '=' or ','")
    } else {
        None
    }
}

fn joined(names: &HashSet<&str>) -> String {
    let mut v: Vec<&str> = names.iter().copied().collect();
    v.sort_unstable();
    v.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = r#"
apiVersion: v1
name: web_events
dimensions:
  - name: geo
    levels: [country, city]
  - name: platform
    levels:
      - name: device
        expr: device_type
      - browser
  - name: gender
filterRules:
  - type: max_dimensions
    n: 3
  - type: not_together
    dims: [geo, platform]
measures:
  - name: pageviews
    agg: sum
    input: pv
  - name: reach
    agg: count_distinct
    input: user_id
  - name: events
    agg: count
includeGlobal: true
"#;

    #[test]
    fn parses_and_validates_reference_spec() {
        let spec = CubeSpec::from_yaml(SPEC).unwrap();
        assert_eq!(spec.dimensions.len(), 3);
        assert_eq!(spec.measures.len(), 3);
        assert!(spec.include_global);

        let platform = &spec.dimensions[1];
        assert_eq!(platform.levels[0].expression(), "device_type");
        assert_eq!(platform.levels[1].expression(), "browser");

        let gender = &spec.dimensions[2];
        assert_eq!(gender.effective_levels()[0].name, "gender");
    }

    #[test]
    fn yaml_round_trip() {
        let spec = CubeSpec::from_yaml(SPEC).unwrap();
        let reparsed = CubeSpec::from_yaml(&spec.to_yaml()).unwrap();
        assert_eq!(spec, reparsed);
    }

    #[test]
    fn sorted_dimensions_are_name_ordered() {
        let spec = CubeSpec::from_yaml(SPEC).unwrap();
        let names: Vec<&str> = spec
            .sorted_dimensions()
            .iter()
            .map(|d| d.name.as_str())
            .collect();
        assert_eq!(names, vec!["gender", "geo", "platform"]);
    }

    #[test]
    fn ypaths_for_values_builds_prefix_hierarchy() {
        let spec = CubeSpec::from_yaml(SPEC).unwrap();
        let geo = &spec.dimensions[0];
        let yps = geo.ypaths_for_values(&[Some("CZ".into()), Some("Prague".into())]);
        assert_eq!(yps.len(), 2);
        assert_eq!(yps[0].to_string(), "/geo/country=CZ");
        assert_eq!(yps[1].to_string(), "/geo/country=CZ/city=Prague");
    }

    #[test]
    fn ypaths_truncate_at_first_missing_level() {
        let spec = CubeSpec::from_yaml(SPEC).unwrap();
        let geo = &spec.dimensions[0];
        let yps = geo.ypaths_for_values(&[Some("CZ".into()), None]);
        assert_eq!(yps.len(), 1);
        assert_eq!(yps[0].to_string(), "/geo/country=CZ");

        let none = geo.ypaths_for_values(&[None, Some("Prague".into())]);
        assert!(none.is_empty());
    }

    #[test]
    fn validation_collects_every_error() {
        let bad = r#"
apiVersion: v2
name: ""
dimensions:
  - name: geo
    levels: [country, country]
  - name: geo
  - name: "a/b"
filterRules:
  - type: contains_dim
    dim: nope
  - type: max_dimensions
    n: 0
measures:
  - name: m
    agg: sum
  - name: m
    agg: count
"#;
        let err = CubeSpec::from_yaml(bad).unwrap_err();
        let CubismError::Validation(errors) = err else {
            panic!("expected validation error")
        };
        let text = errors.join("\n");
        assert!(text.contains("apiVersion 'v2'"));
        assert!(text.contains("'name' must not be empty"));
        assert!(text.contains("duplicate level name 'country'"));
        assert!(text.contains("duplicate dimension name 'geo'"));
        assert!(text.contains("must not contain"));
        assert!(text.contains("unknown dimension 'nope'"));
        assert!(text.contains("max_dimensions n must be at least 1"));
        assert!(text.contains("requires an 'input'"));
        assert!(text.contains("duplicate measure name 'm'"));
    }

    #[test]
    fn unknown_fields_are_rejected_with_a_readable_error() {
        let typo = r#"
apiVersion: v1
name: x
dimenssions:
  - name: geo
"#;
        let err = CubeSpec::from_yaml(typo).unwrap_err();
        assert!(matches!(err, CubismError::SpecParse(_)));
        assert!(err.to_string().contains("dimenssions"));
    }

    #[test]
    fn parses_v2alpha1_temporal_spec_and_hashes_semantics() {
        let yaml = r#"
apiVersion: cubism/v2alpha1
name: temporal_events
dimensions:
  - name: geo
measures:
  - name: latency_p50
    agg: quantile
    input: latency_ms
    state:
      quantileBinWidth: 0.5
  - name: latency_variance
    agg: variance
    input: latency_ms
temporal:
  eventTime: occurred_at
  ingestionTime: ingested_at
  baseResolution: 5m
  origin: unix
  timezone: UTC
  allowedLateness: 2h
  rollups: [1h, 1d]
  retention: 90d
"#;
        let spec = CubeSpec::from_yaml(yaml).unwrap();
        assert_eq!(spec.api_version, API_VERSION_V2_ALPHA1);
        assert_eq!(spec.temporal.as_ref().unwrap().event_time, "occurred_at");
        assert_eq!(
            spec.spec_hash().unwrap(),
            CubeSpec::from_yaml(&spec.to_yaml())
                .unwrap()
                .spec_hash()
                .unwrap()
        );

        let changed = yaml.replace("allowedLateness: 2h", "allowedLateness: 3h");
        assert_ne!(
            spec.spec_hash().unwrap(),
            CubeSpec::from_yaml(&changed).unwrap().spec_hash().unwrap()
        );
    }

    #[test]
    fn temporal_spec_rejects_calendar_and_current_topk_semantics() {
        let yaml = r#"
apiVersion: cubism/v2alpha1
name: temporal_events
dimensions:
  - name: geo
measures:
  - name: top
    agg: top_k
    input: page
    by: score
temporal:
  eventTime: occurred_at
  baseResolution: calendar_month
  allowedLateness: 1h
"#;
        let error = CubeSpec::from_yaml(yaml).unwrap_err().to_string();
        assert!(error.contains("calendar resolution"));
        assert!(error.contains("top_k is not supported"));
    }

    #[test]
    fn v1_does_not_infer_or_accept_temporal_configuration() {
        let yaml = r#"
apiVersion: v1
name: static
dimensions:
  - name: time
measures:
  - name: events
    agg: count
temporal:
  eventTime: occurred_at
  baseResolution: 1h
  allowedLateness: 1h
"#;
        let error = CubeSpec::from_yaml(yaml).unwrap_err().to_string();
        assert!(error.contains("does not support a temporal section"));
    }
}
