//! Property tests for the core algebra:
//! - XUnit string form parse/format round-trip
//! - binary key encode/decode round-trip and key canonicality
//! - pruned lattice generation ≡ generate-everything-then-filter

use cubism_core::encoding::{decode_xunit, encode_xunit, XUnitDictionary};
use cubism_core::lattice::{generate_xunits, generate_xunits_unpruned};
use cubism_core::rules::{include_xunit, FilterRule};
use cubism_core::sketch::KmvSketch;
use cubism_core::{XUnit, YPath};
use proptest::prelude::*;

/// Values avoid the legacy scrub sentinels (`___`, `###`, `+++`), which are
/// documented-lossy; structural chars `/ = ,` themselves are fair game since
/// scrubbing escapes them.
fn value_strategy() -> impl Strategy<Value = String> {
    "[A-Za-z0-9 ./=,-]{1,12}".prop_filter("no scrub sentinels", |s| {
        !s.contains("___") && !s.contains("###") && !s.contains("+++")
    })
}

fn ident_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,7}".prop_filter("no sentinel", |s| !s.contains("___"))
}

/// A YPath with distinct-ish attribute names and a mix of Some/None values.
fn ypath_strategy() -> impl Strategy<Value = YPath> {
    (
        ident_strategy(),
        prop::collection::vec((ident_strategy(), prop::option::weighted(0.9, value_strategy())), 1..4),
    )
        .prop_map(|(dim, attributes)| YPath { dim, attributes })
}

/// An XUnit with unique dimensions (the model's invariant: at most one YPath
/// per dimension).
fn xunit_strategy() -> impl Strategy<Value = XUnit> {
    prop::collection::vec(ypath_strategy(), 0..4).prop_map(|mut ypaths| {
        ypaths.sort_by(|a, b| a.dim.cmp(&b.dim));
        ypaths.dedup_by(|a, b| a.dim == b.dim);
        XUnit::new(ypaths)
    })
}

/// Per-dimension YPath alternatives shaped like real hierarchy prefixes.
fn per_dimension_strategy() -> impl Strategy<Value = Vec<Vec<YPath>>> {
    prop::collection::vec(
        (ident_strategy(), prop::collection::vec((ident_strategy(), value_strategy()), 0..3)),
        1..4,
    )
    .prop_map(|dims| {
        dims.into_iter()
            .enumerate()
            .map(|(i, (dim, levels))| {
                let dim = format!("{dim}{i}"); // keep dimensions distinct
                let mut out = Vec::new();
                let mut yp = YPath::new(dim);
                for (name, value) in levels {
                    yp = yp.with_attribute(name, value);
                    out.push(yp.clone());
                }
                out
            })
            .collect()
    })
}

fn rules_strategy() -> impl Strategy<Value = Vec<FilterRule>> {
    let dim = "[a-z][a-z0-9_]{0,7}[0-9]".prop_map(String::from);
    let leaf = prop_oneof![
        (1usize..4).prop_map(|n| FilterRule::MaxDimensions { n }),
        (1usize..4).prop_map(|n| FilterRule::MinDimensions { n }),
        dim.clone().prop_map(|d| FilterRule::ContainsDim { dim: d }),
        dim.clone().prop_map(|d| FilterRule::TopLevel { dim: d }),
        dim.clone().prop_map(|d| FilterRule::NotAlone { dim: d }),
        (dim.clone(), dim.clone())
            .prop_map(|(a, b)| FilterRule::NotTogether { dims: vec![a, b] }),
    ];
    let combined = leaf.clone().prop_recursive(2, 8, 3, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 1..3).prop_map(|rules| FilterRule::And { rules }),
            prop::collection::vec(inner, 1..3).prop_map(|rules| FilterRule::Or { rules }),
        ]
    });
    prop::collection::vec(combined, 0..4)
}

proptest! {
    #[test]
    fn string_round_trip(x in xunit_strategy()) {
        let s = x.to_string();
        let parsed: XUnit = s.parse().unwrap();
        prop_assert_eq!(parsed, x.normalize());
    }

    #[test]
    fn normalize_is_idempotent(x in xunit_strategy()) {
        let once = x.clone().normalize();
        prop_assert_eq!(once.clone().normalize(), once);
    }

    #[test]
    fn binary_round_trip(x in xunit_strategy()) {
        let mut dict = XUnitDictionary::new();
        let key = encode_xunit(&x, &mut dict);
        prop_assert_eq!(decode_xunit(&key, &dict).unwrap(), x.normalize());
    }

    #[test]
    fn key_is_canonical_under_ypath_order(x in xunit_strategy()) {
        let mut dict = XUnitDictionary::new();
        let mut shuffled = x.clone();
        shuffled.ypaths.reverse();
        prop_assert_eq!(encode_xunit(&x, &mut dict), encode_xunit(&shuffled, &mut dict));
    }

    #[test]
    fn pruned_generation_equals_filtered_reference(
        per_dim in per_dimension_strategy(),
        rules in rules_strategy(),
        include_global in any::<bool>(),
    ) {
        let pruned = generate_xunits(&per_dim, &rules, include_global);

        let mut reference: Vec<XUnit> = generate_xunits_unpruned(&per_dim)
            .into_iter()
            .filter(|x| include_xunit(&rules, x))
            .map(XUnit::normalize)
            .collect();
        if include_global {
            reference.push(XUnit::global());
        }

        prop_assert_eq!(pruned, reference);
    }

    // --- Sketch merge algebra: the properties the whole system leans on. ---

    #[test]
    fn kmv_merge_is_commutative_and_associative(
        a in prop::collection::vec(any::<u64>(), 0..200),
        b in prop::collection::vec(any::<u64>(), 0..200),
        c in prop::collection::vec(any::<u64>(), 0..200),
        k in 8u32..64,
    ) {
        let (a, b, c) = (
            KmvSketch::from_hashes(k, a),
            KmvSketch::from_hashes(k, b),
            KmvSketch::from_hashes(k, c),
        );
        prop_assert_eq!(a.merge(&b), b.merge(&a));
        prop_assert_eq!(a.merge(&b).merge(&c), a.merge(&b.merge(&c)));
        // Identity and idempotence.
        prop_assert_eq!(a.merge(&KmvSketch::new(k)), a.clone());
        prop_assert_eq!(a.merge(&a), a);
    }

    #[test]
    fn kmv_bytes_round_trip(
        hashes in prop::collection::vec(any::<u64>(), 0..200),
        k in 8u32..64,
    ) {
        let s = KmvSketch::from_hashes(k, hashes);
        prop_assert_eq!(KmvSketch::from_bytes(&s.to_bytes()).unwrap(), s);
    }
}
