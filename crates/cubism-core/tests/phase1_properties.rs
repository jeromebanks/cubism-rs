use cubism_core::aggregate_state::{AggregateState, AverageState, QuantileState, VarianceState};
use cubism_core::encoding::{
    CanonicalAttribute, CanonicalValue, CanonicalXUnit, CanonicalYPath, canonical_xunit_content_id,
    decode_canonical_xunit, encode_canonical_xunit,
};
use cubism_core::{BucketOrigin, EventTime, FixedResolution};
use proptest::prelude::*;

proptest! {
    #[test]
    fn every_timestamp_belongs_to_exactly_one_adjacent_fixed_bucket(
        timestamp in -8_000_000_000_000i64..8_000_000_000_000,
        origin in -1_000_000_000i64..1_000_000_000,
        width in 1i64..1_000_000_000,
    ) {
        let resolution = FixedResolution::from_micros(width).unwrap();
        let timestamp = EventTime::from_unix_micros(timestamp);
        let bucket = resolution
            .bucket(timestamp, BucketOrigin::from_unix_micros(origin))
            .unwrap();
        prop_assert!(bucket.contains(timestamp));
        prop_assert_eq!(
            bucket.end.unix_micros() - bucket.start.unix_micros(),
            width
        );

        let previous_timestamp = EventTime::from_unix_micros(bucket.start.unix_micros() - 1);
        let previous = resolution
            .bucket(previous_timestamp, BucketOrigin::from_unix_micros(origin))
            .unwrap();
        prop_assert_eq!(previous.end.unix_micros(), bucket.start.unix_micros());

        let next_timestamp = EventTime::from_unix_micros(bucket.end.unix_micros());
        let next = resolution
            .bucket(next_timestamp, BucketOrigin::from_unix_micros(origin))
            .unwrap();
        prop_assert_eq!(next.start.unix_micros(), bucket.end.unix_micros());
    }

    #[test]
    fn canonical_identity_ignores_ypath_input_order(
        left in "[a-z]{1,8}",
        right in "[a-z]{1,8}",
        value in ".*",
    ) {
        prop_assume!(left != right);
        let first = CanonicalYPath {
            dim: left,
            attributes: vec![CanonicalAttribute {
                name: "value".into(),
                value: CanonicalValue::String(value),
            }],
        };
        let second = CanonicalYPath {
            dim: right,
            attributes: vec![CanonicalAttribute {
                name: "null".into(),
                value: CanonicalValue::Null,
            }],
        };
        let forward = CanonicalXUnit { ypaths: vec![first.clone(), second.clone()] };
        let reverse = CanonicalXUnit { ypaths: vec![second, first] };
        prop_assert_eq!(
            encode_canonical_xunit(&forward).unwrap(),
            encode_canonical_xunit(&reverse).unwrap()
        );
        prop_assert_eq!(
            canonical_xunit_content_id(&forward).unwrap(),
            canonical_xunit_content_id(&reverse).unwrap()
        );
        let bytes = encode_canonical_xunit(&forward).unwrap();
        prop_assert_eq!(
            encode_canonical_xunit(&decode_canonical_xunit(&bytes).unwrap()).unwrap(),
            bytes
        );
    }

    #[test]
    fn partitioned_numeric_accumulation_matches_single_pass_with_tolerance(
        values in prop::collection::vec(-1_000_000f64..1_000_000f64, 0..200),
        split_seed in any::<usize>(),
    ) {
        let split = if values.is_empty() { 0 } else { split_seed % (values.len() + 1) };

        let mut average_left = AverageState::new();
        let mut average_right = AverageState::new();
        let mut average_direct = AverageState::new();
        let mut variance_left = VarianceState::new();
        let mut variance_right = VarianceState::new();
        let mut variance_direct = VarianceState::new();
        let mut quantile_left = QuantileState::new(0.25).unwrap();
        let mut quantile_right = QuantileState::new(0.25).unwrap();
        let mut quantile_direct = QuantileState::new(0.25).unwrap();

        for (index, value) in values.iter().copied().enumerate() {
            average_direct.accumulate(value).unwrap();
            variance_direct.accumulate(value).unwrap();
            quantile_direct.accumulate(value).unwrap();
            if index < split {
                average_left.accumulate(value).unwrap();
                variance_left.accumulate(value).unwrap();
                quantile_left.accumulate(value).unwrap();
            } else {
                average_right.accumulate(value).unwrap();
                variance_right.accumulate(value).unwrap();
                quantile_right.accumulate(value).unwrap();
            }
        }

        let average_merged = average_left.merge(&average_right).unwrap();
        let variance_merged = variance_left.merge(&variance_right).unwrap();
        let quantile_merged = quantile_left.merge(&quantile_right).unwrap();

        match (average_merged.present(), average_direct.present()) {
            (Some(merged), Some(direct)) => {
                let scale = direct.abs().max(1.0);
                prop_assert!((merged - direct).abs() <= 1e-10 * scale);
            }
            (None, None) => {}
            other => prop_assert!(false, "average presence mismatch: {other:?}"),
        }
        match (
            variance_merged.population_variance(),
            variance_direct.population_variance(),
        ) {
            (Some(merged), Some(direct)) => {
                let scale = direct.abs().max(1.0);
                prop_assert!((merged - direct).abs() <= 1e-10 * scale);
            }
            (None, None) => {}
            other => prop_assert!(false, "variance presence mismatch: {other:?}"),
        }
        prop_assert_eq!(quantile_merged, quantile_direct);
    }
}
