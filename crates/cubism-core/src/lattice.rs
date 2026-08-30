//! Cube-lattice generation: explode one row's dimension coordinates into
//! every lattice cell (XUnit) the row belongs to.
//!
//! Ported from the legacy `Qubism.generateXUnits` fold, with the key
//! improvement the legacy code deferred (`generate everything, then filter`):
//! the `MaxDimensions` bound derived from the rule conjunction stops
//! candidates from being *extended* past the bound during generation, so the
//! combinatorial blow-up is capped instead of materialized. All other rules
//! are applied per-candidate at emit time.
//!
//! Within one dimension, hierarchy levels are alternatives (a cell never
//! contains `/geo/country=CZ` *and* `/geo/country=CZ/city=Prague`); across
//! dimensions, cells combine. An empty per-dimension YPath list (e.g. a null
//! dimension value in the row) simply contributes nothing, matching legacy
//! behavior. The global rollup, when requested, is appended *after*
//! filtering — it bypasses the rules, exactly as the legacy library did.

use crate::rules::{self, FilterRule};
use crate::ypath::XUnit;
use crate::ypath::YPath;

/// Generate the pruned lattice for one row.
///
/// `per_dimension` holds, for each dimension (in canonical dimension order),
/// the alternative YPaths the row produces for it — typically the hierarchy
/// prefixes `[/geo/country=CZ, /geo/country=CZ/city=Prague]`.
pub fn generate_xunits(
    per_dimension: &[Vec<YPath>],
    filter_rules: &[FilterRule],
    include_global: bool,
) -> Vec<XUnit> {
    let bound = rules::max_dimensions_bound(filter_rules);

    let mut candidates: Vec<XUnit> = Vec::new();
    for dim_ypaths in per_dimension {
        if dim_ypaths.is_empty() {
            continue;
        }
        let mut new_units: Vec<XUnit> = dim_ypaths
            .iter()
            .map(|yp| XUnit::new(vec![yp.clone()]))
            .collect();
        for existing in &candidates {
            if bound.is_some_and(|b| existing.num_dimensions() >= b) {
                continue; // MaxDimensions is monotone: extensions can never re-qualify.
            }
            for yp in dim_ypaths {
                new_units.push(existing.clone().with_ypath(yp.clone()));
            }
        }
        candidates.append(&mut new_units);
    }

    let mut result: Vec<XUnit> = candidates
        .into_iter()
        .filter(|x| rules::include_xunit(filter_rules, x))
        .map(XUnit::normalize)
        .collect();

    if include_global {
        result.push(XUnit::global());
    }
    result
}

/// Reference implementation: full lattice, no pruning during generation.
/// Exists for differential/property testing against [`generate_xunits`].
pub fn generate_xunits_unpruned(per_dimension: &[Vec<YPath>]) -> Vec<XUnit> {
    let mut candidates: Vec<XUnit> = Vec::new();
    for dim_ypaths in per_dimension {
        let new_units: Vec<XUnit> = dim_ypaths
            .iter()
            .map(|yp| XUnit::new(vec![yp.clone()]))
            .chain(candidates.iter().flat_map(|existing| {
                dim_ypaths
                    .iter()
                    .map(|yp| existing.clone().with_ypath(yp.clone()))
            }))
            .collect();
        candidates.extend(new_units);
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `/dim/l1=v .. /ln=v` hierarchy prefixes, `depth` levels.
    fn hierarchy(dim: &str, levels: &[&str]) -> Vec<YPath> {
        let mut out = Vec::new();
        let mut yp = YPath::new(dim);
        for level in levels {
            yp = yp.with_attribute(*level, format!("{level}_v"));
            out.push(yp.clone());
        }
        out
    }

    fn test_dims() -> Vec<Vec<YPath>> {
        // Mirrors the legacy XUnitSpec fixture: platform(device,browser),
        // geo(country,city), gender — sorted by dimension name as
        // XUnitDefinition.ypathExtractors does.
        vec![
            hierarchy("gender", &["gender"]),
            hierarchy("geo", &["country", "city"]),
            hierarchy("platform", &["device", "browser"]),
        ]
    }

    #[test]
    fn unpruned_lattice_size_matches_legacy_spec() {
        // Legacy XUnitSpec: (2+1)(2+1)(1+1) − 1 = 17 cells without global.
        let x = generate_xunits_unpruned(&test_dims());
        assert_eq!(x.len(), 17);
    }

    #[test]
    fn max_dimensions_three_matches_legacy_17_and_18_with_global() {
        let rules = vec![FilterRule::MaxDimensions { n: 3 }];
        assert_eq!(generate_xunits(&test_dims(), &rules, false).len(), 17);
        assert_eq!(generate_xunits(&test_dims(), &rules, true).len(), 18);
    }

    #[test]
    fn exactly_three_dims_matches_legacy_three_dim_rule() {
        // Legacy ThreeDimRule ("xunit.ypaths.size == 3") over the same
        // fixture yields 4 XUnits: 2 platform levels × 2 geo levels × 1 gender.
        let rules = vec![FilterRule::And {
            rules: vec![
                FilterRule::MinDimensions { n: 3 },
                FilterRule::MaxDimensions { n: 3 },
            ],
        }];
        let x = generate_xunits(&test_dims(), &rules, false);
        assert_eq!(x.len(), 4);
        assert!(x.iter().all(|u| u.num_dimensions() == 3));
    }

    #[test]
    fn empty_dimension_contributes_nothing() {
        let dims = vec![hierarchy("gender", &["gender"]), Vec::new()];
        let x = generate_xunits(&dims, &[], false);
        assert_eq!(x.len(), 1);
    }

    #[test]
    fn global_bypasses_filters() {
        // A rule the global cell fails (min 1 dim) — global still appears.
        let rules = vec![FilterRule::MinDimensions { n: 1 }];
        let x = generate_xunits(&test_dims(), &rules, true);
        assert!(x.iter().any(XUnit::is_global));
    }

    #[test]
    fn pruned_equals_filtered_unpruned() {
        let rules = vec![
            FilterRule::MaxDimensions { n: 2 },
            FilterRule::NotTogether {
                dims: vec!["geo".into(), "platform".into()],
            },
        ];
        let pruned = generate_xunits(&test_dims(), &rules, false);
        let reference: Vec<XUnit> = generate_xunits_unpruned(&test_dims())
            .into_iter()
            .filter(|x| crate::rules::include_xunit(&rules, x))
            .map(XUnit::normalize)
            .collect();
        assert_eq!(pruned, reference);
    }
}
