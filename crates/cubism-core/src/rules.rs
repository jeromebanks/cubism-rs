//! Filter rules prune the cube lattice: without them, a row with `d`
//! dimensions of hierarchy depth `h_i` explodes into `∏(h_i + 1) − 1` cells.
//!
//! Semantics ported from the legacy `FilterRule.scala`, with two deliberate
//! changes: the always-true `ContainsAllDims` bug is fixed (it now actually
//! inspects the XUnit), and `MinDimensions` is added (the legacy tests used a
//! custom "exactly N dims" rule; `And[Min, Max]` expresses that declaratively).
//!
//! An XUnit is included when **every** rule's `should_include` passes
//! (empty rule list ⇒ include everything). The legacy `mustInclude` override
//! was dropped: no legacy rule ever implemented it.

use crate::ypath::XUnit;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FilterRule {
    /// Include XUnits with at most `n` dimensions.
    MaxDimensions {
        n: usize,
    },
    /// Include XUnits with at least `n` dimensions.
    MinDimensions {
        n: usize,
    },
    /// Include only XUnits containing this dimension.
    ContainsDim {
        dim: String,
    },
    /// Include only XUnits containing all of these dimensions.
    ContainsAllDims {
        dims: Vec<String>,
    },
    /// Include this dimension only as a single-dimension (top-level) XUnit.
    TopLevel {
        dim: String,
    },
    /// Exclude XUnits containing all of these dimensions together.
    NotTogether {
        dims: Vec<String>,
    },
    /// Exclude XUnits combining `dim` with a YPath for `other_dim` that has
    /// drilled down to attribute `attr`.
    NotWithAttribute {
        dim: String,
        other_dim: String,
        attr: String,
    },
    /// Exclude this dimension as a standalone single-dimension XUnit.
    NotAlone {
        dim: String,
    },
    /// Include only XUnits containing this dimension (alias kept for spec
    /// readability; same predicate as `ContainsDim`).
    OnlyWith {
        dim: String,
    },
    And {
        rules: Vec<FilterRule>,
    },
    Or {
        rules: Vec<FilterRule>,
    },
}

impl FilterRule {
    pub fn should_include(&self, xunit: &XUnit) -> bool {
        match self {
            FilterRule::MaxDimensions { n } => xunit.num_dimensions() <= *n,
            FilterRule::MinDimensions { n } => xunit.num_dimensions() >= *n,
            FilterRule::ContainsDim { dim } => xunit.contains_dimension(dim),
            FilterRule::ContainsAllDims { dims } => {
                dims.iter().all(|d| xunit.contains_dimension(d))
            }
            FilterRule::TopLevel { dim } => {
                xunit.num_dimensions() == 1 && xunit.contains_dimension(dim)
            }
            FilterRule::NotTogether { dims } => !dims.iter().all(|d| xunit.contains_dimension(d)),
            FilterRule::NotWithAttribute {
                dim,
                other_dim,
                attr,
            } => {
                if xunit.contains_dimension(dim) {
                    match xunit.for_dimension(other_dim) {
                        Some(yp) => !yp.attributes.iter().any(|(name, _)| name == attr),
                        None => true,
                    }
                } else {
                    true
                }
            }
            FilterRule::NotAlone { dim } => {
                !(xunit.num_dimensions() == 1 && xunit.contains_dimension(dim))
            }
            FilterRule::OnlyWith { dim } => xunit.contains_dimension(dim),
            FilterRule::And { rules } => rules.iter().all(|r| r.should_include(xunit)),
            FilterRule::Or { rules } => rules.iter().any(|r| r.should_include(xunit)),
        }
    }

    /// All dimension names this rule references (for spec validation).
    pub fn referenced_dims(&self) -> Vec<&str> {
        match self {
            FilterRule::MaxDimensions { .. } | FilterRule::MinDimensions { .. } => Vec::new(),
            FilterRule::ContainsDim { dim }
            | FilterRule::TopLevel { dim }
            | FilterRule::NotAlone { dim }
            | FilterRule::OnlyWith { dim } => vec![dim],
            FilterRule::ContainsAllDims { dims } | FilterRule::NotTogether { dims } => {
                dims.iter().map(String::as_str).collect()
            }
            FilterRule::NotWithAttribute { dim, other_dim, .. } => vec![dim, other_dim],
            FilterRule::And { rules } | FilterRule::Or { rules } => {
                rules.iter().flat_map(|r| r.referenced_dims()).collect()
            }
        }
    }
}

/// Conjunction over all rules; empty rule list includes everything.
pub fn include_xunit(rules: &[FilterRule], xunit: &XUnit) -> bool {
    rules.iter().all(|r| r.should_include(xunit))
}

/// The tightest dimension-count bound derivable from the rule conjunction,
/// used to stop extending lattice candidates early during generation.
/// `MaxDimensions` is monotone (every superset of an excluded cell is also
/// excluded), so growth-bounding with it is sound. Only conjunctive contexts
/// (top level and `And`) are inspected — a `MaxDimensions` inside an `Or`
/// bounds nothing.
pub fn max_dimensions_bound(rules: &[FilterRule]) -> Option<usize> {
    rules
        .iter()
        .filter_map(|r| match r {
            FilterRule::MaxDimensions { n } => Some(*n),
            FilterRule::And { rules } => max_dimensions_bound(rules),
            _ => None,
        })
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ypath::YPath;

    fn xu(dims: &[&str]) -> XUnit {
        XUnit::new(
            dims.iter()
                .map(|d| YPath::new(*d).with_attribute(*d, "v"))
                .collect(),
        )
    }

    #[test]
    fn max_and_min_dimensions() {
        let max2 = FilterRule::MaxDimensions { n: 2 };
        assert!(max2.should_include(&xu(&["a", "b"])));
        assert!(!max2.should_include(&xu(&["a", "b", "c"])));
        assert!(max2.should_include(&XUnit::global()));

        let min2 = FilterRule::MinDimensions { n: 2 };
        assert!(!min2.should_include(&xu(&["a"])));
        assert!(min2.should_include(&xu(&["a", "b"])));
    }

    #[test]
    fn not_together() {
        let rule = FilterRule::NotTogether {
            dims: vec!["a".into(), "b".into()],
        };
        assert!(!rule.should_include(&xu(&["a", "b", "c"])));
        assert!(rule.should_include(&xu(&["a", "c"])));
        assert!(rule.should_include(&xu(&["b"])));
    }

    #[test]
    fn contains_all_dims_actually_checks_the_xunit() {
        // The legacy implementation was `dims.forall(dims.contains)` — always true.
        let rule = FilterRule::ContainsAllDims {
            dims: vec!["a".into(), "b".into()],
        };
        assert!(rule.should_include(&xu(&["a", "b", "c"])));
        assert!(!rule.should_include(&xu(&["a", "c"])));
    }

    #[test]
    fn top_level_and_not_alone() {
        let top = FilterRule::TopLevel { dim: "a".into() };
        assert!(top.should_include(&xu(&["a"])));
        assert!(!top.should_include(&xu(&["a", "b"])));
        assert!(!top.should_include(&xu(&["b"])));

        let alone = FilterRule::NotAlone { dim: "a".into() };
        assert!(!alone.should_include(&xu(&["a"])));
        assert!(alone.should_include(&xu(&["a", "b"])));
        assert!(alone.should_include(&xu(&["b"])));
    }

    #[test]
    fn not_with_attribute() {
        let rule = FilterRule::NotWithAttribute {
            dim: "gender".into(),
            other_dim: "geo".into(),
            attr: "city".into(),
        };
        let deep = XUnit::new(vec![
            YPath::new("geo")
                .with_attribute("country", "CZ")
                .with_attribute("city", "Prague"),
            YPath::new("gender").with_attribute("gender", "F"),
        ]);
        let shallow = XUnit::new(vec![
            YPath::new("geo").with_attribute("country", "CZ"),
            YPath::new("gender").with_attribute("gender", "F"),
        ]);
        assert!(!rule.should_include(&deep));
        assert!(rule.should_include(&shallow));
    }

    #[test]
    fn and_or_combinators() {
        let exactly2 = FilterRule::And {
            rules: vec![
                FilterRule::MaxDimensions { n: 2 },
                FilterRule::MinDimensions { n: 2 },
            ],
        };
        assert!(exactly2.should_include(&xu(&["a", "b"])));
        assert!(!exactly2.should_include(&xu(&["a"])));
        assert!(!exactly2.should_include(&xu(&["a", "b", "c"])));

        let either = FilterRule::Or {
            rules: vec![
                FilterRule::ContainsDim { dim: "a".into() },
                FilterRule::ContainsDim { dim: "b".into() },
            ],
        };
        assert!(either.should_include(&xu(&["b", "c"])));
        assert!(!either.should_include(&xu(&["c"])));
    }

    #[test]
    fn empty_rules_include_everything() {
        assert!(include_xunit(&[], &xu(&["a", "b", "c"])));
    }

    #[test]
    fn bound_extraction() {
        assert_eq!(max_dimensions_bound(&[]), None);
        assert_eq!(
            max_dimensions_bound(&[
                FilterRule::MaxDimensions { n: 3 },
                FilterRule::And {
                    rules: vec![FilterRule::MaxDimensions { n: 2 }]
                },
                FilterRule::Or {
                    rules: vec![FilterRule::MaxDimensions { n: 1 }]
                },
            ]),
            Some(2)
        );
    }

    #[test]
    fn serde_yaml_round_trip() {
        let yaml = r#"
- type: max_dimensions
  n: 3
- type: not_together
  dims: [geo, platform]
- type: or
  rules:
    - type: contains_dim
      dim: gender
    - type: top_level
      dim: geo
"#;
        let rules: Vec<FilterRule> = serde_norway::from_str(yaml).unwrap();
        assert_eq!(rules.len(), 3);
        let back = serde_norway::to_string(&rules).unwrap();
        let reparsed: Vec<FilterRule> = serde_norway::from_str(&back).unwrap();
        assert_eq!(rules, reparsed);
    }
}
