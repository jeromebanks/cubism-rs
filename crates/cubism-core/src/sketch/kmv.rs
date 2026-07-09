//! KMV (k-minimum-values) distinct-count sketch.
//!
//! Keep the `k` smallest 64-bit hashes seen; the k-th smallest value
//! estimates the density of the hashed set, hence its cardinality. Because
//! two sketches merge by a sorted dedup-merge (also capped at k), KMV
//! supports union — and via inclusion-exclusion, intersection and Jaccard
//! similarity — over pre-aggregated data. That set algebra is Cubism's
//! signature capability.
//!
//! Ported from the legacy `KmvSketch.scala` with two deliberate changes,
//! both part of byte-format v1:
//! - **Hash**: xxh3-64 with seed 0 over the value's bytes (legacy used MD5's
//!   first 8 bytes). The hash is part of the persisted format contract.
//! - **Estimator**: unbiased `(k-1)/U` where `U = kth_min / 2^64` (legacy
//!   used the biased `k/U` form).
//!
//! Error: relative standard error ≈ `1/sqrt(k-2)` — ~3.1% at the default
//! k=1024.

use crate::error::CubismError;
use xxhash_rust::xxh3::xxh3_64;

pub const DEFAULT_SKETCH_SIZE: u32 = 1024;

const MAGIC: &[u8; 3] = b"KMV";
const VERSION: u8 = 1;
const HEADER_LEN: usize = 3 + 1 + 4 + 4; // magic + version + k + n

/// Hash a value into the sketch's hash space. Fixed seed — this function is
/// part of the persisted-format contract.
pub fn hash_value(bytes: &[u8]) -> u64 {
    xxh3_64(bytes)
}

#[derive(Debug, Clone, PartialEq)]
pub struct KmvSketch {
    k: u32,
    /// Sorted ascending (unsigned), distinct, `len <= k`.
    hashes: Vec<u64>,
}

impl KmvSketch {
    pub fn new(k: u32) -> Self {
        assert!(k >= 8, "sketch size k must be at least 8");
        KmvSketch { k, hashes: Vec::new() }
    }

    pub fn k(&self) -> u32 {
        self.k
    }

    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    pub fn insert(&mut self, value: &[u8]) {
        self.insert_hash(hash_value(value));
    }

    pub fn insert_hash(&mut self, hash: u64) {
        match self.hashes.binary_search(&hash) {
            Ok(_) => {} // already present
            Err(pos) => {
                if pos < self.k as usize {
                    self.hashes.insert(pos, hash);
                    self.hashes.truncate(self.k as usize);
                }
            }
        }
    }

    /// Bulk-build from an iterator of hashes: sort/dedup once instead of
    /// per-item insertion. The efficient path for batch accumulation.
    pub fn from_hashes(k: u32, hashes: impl IntoIterator<Item = u64>) -> Self {
        let mut hashes: Vec<u64> = hashes.into_iter().collect();
        hashes.sort_unstable();
        hashes.dedup();
        hashes.truncate(k as usize);
        KmvSketch { k, hashes }
    }

    /// Union merge. Result capacity is `min(k_l, k_r)` — a sketch can only
    /// be trusted up to the smaller k, exactly as the legacy library chose.
    pub fn merge(&self, other: &KmvSketch) -> KmvSketch {
        let k = self.k.min(other.k) as usize;
        let mut merged = Vec::with_capacity((self.len() + other.len()).min(k));
        let (mut i, mut j) = (0, 0);
        while merged.len() < k {
            match (self.hashes.get(i), other.hashes.get(j)) {
                (Some(&a), Some(&b)) => {
                    if a < b {
                        merged.push(a);
                        i += 1;
                    } else if b < a {
                        merged.push(b);
                        j += 1;
                    } else {
                        merged.push(a);
                        i += 1;
                        j += 1;
                    }
                }
                (Some(&a), None) => {
                    merged.push(a);
                    i += 1;
                }
                (None, Some(&b)) => {
                    merged.push(b);
                    j += 1;
                }
                (None, None) => break,
            }
        }
        KmvSketch { k: k as u32, hashes: merged }
    }

    /// Estimated distinct count. Exact while the sketch is under-full.
    pub fn estimate(&self) -> f64 {
        let n = self.hashes.len();
        if n < self.k as usize {
            n as f64
        } else {
            let kth = *self.hashes.last().expect("full sketch is nonempty");
            let u = kth as f64 / u64::MAX as f64;
            (n as f64 - 1.0) / u
        }
    }

    /// Estimated |self ∪ other|.
    pub fn union_estimate(&self, other: &KmvSketch) -> f64 {
        self.merge(other).estimate()
    }

    /// Estimated |self ∩ other| via inclusion-exclusion, clamped at 0.
    pub fn intersection_estimate(&self, other: &KmvSketch) -> f64 {
        (self.estimate() + other.estimate() - self.union_estimate(other)).max(0.0)
    }

    /// Estimated Jaccard similarity |A∩B| / |A∪B|.
    pub fn jaccard(&self, other: &KmvSketch) -> f64 {
        let union = self.union_estimate(other);
        if union == 0.0 { 0.0 } else { self.intersection_estimate(other) / union }
    }

    /// Serialize as format v1: `"KMV" ver:u8 k:u32-le n:u32-le hash:u64-le*n`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.hashes.len() * 8);
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&self.k.to_le_bytes());
        out.extend_from_slice(&(self.hashes.len() as u32).to_le_bytes());
        for h in &self.hashes {
            out.extend_from_slice(&h.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CubismError> {
        let err = |reason: String| CubismError::Decode(format!("KMV sketch: {reason}"));
        if bytes.len() < HEADER_LEN {
            return Err(err(format!("{} bytes is shorter than the header", bytes.len())));
        }
        if &bytes[0..3] != MAGIC {
            return Err(err("bad magic".into()));
        }
        if bytes[3] != VERSION {
            return Err(err(format!("unsupported version {}", bytes[3])));
        }
        let k = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let n = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        if bytes.len() != HEADER_LEN + n * 8 {
            return Err(err(format!("expected {} bytes for n={n}, got {}", HEADER_LEN + n * 8, bytes.len())));
        }
        if n > k as usize {
            return Err(err(format!("n={n} exceeds k={k}")));
        }
        let hashes: Vec<u64> = bytes[HEADER_LEN..]
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        if !hashes.is_sorted() || hashes.windows(2).any(|w| w[0] == w[1]) {
            return Err(err("hashes are not sorted-distinct".into()));
        }
        Ok(KmvSketch { k, hashes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sketch_of(k: u32, items: impl IntoIterator<Item = String>) -> KmvSketch {
        KmvSketch::from_hashes(k, items.into_iter().map(|s| hash_value(s.as_bytes())))
    }

    #[test]
    fn exact_while_underfull() {
        let s = sketch_of(1024, (0..500).map(|i| format!("item{i}")));
        assert_eq!(s.estimate(), 500.0);
    }

    #[test]
    fn insert_dedupes() {
        let mut s = KmvSketch::new(64);
        for _ in 0..10 {
            s.insert(b"same");
        }
        s.insert(b"other");
        assert_eq!(s.estimate(), 2.0);
    }

    #[test]
    fn estimate_within_error_bounds_when_full() {
        // k=1024, n=100k distinct: relative std error ~1/sqrt(k-2) ≈ 3.1%.
        // Deterministic hash, so this is a fixed sample; allow 4 sigma.
        let n = 100_000;
        let s = sketch_of(1024, (0..n).map(|i| format!("user_{i}")));
        let est = s.estimate();
        let rel_err = (est - n as f64).abs() / n as f64;
        assert!(rel_err < 0.125, "estimate {est} vs {n}: rel err {rel_err:.4}");
    }

    #[test]
    fn merge_equals_union_build() {
        let a = sketch_of(256, (0..5_000).map(|i| format!("a{i}")));
        let b = sketch_of(256, (0..5_000).map(|i| format!("b{i}")));
        let merged = a.merge(&b);
        let direct = sketch_of(
            256,
            (0..5_000).flat_map(|i| [format!("a{i}"), format!("b{i}")]),
        );
        assert_eq!(merged, direct);
    }

    #[test]
    fn merge_uses_min_k() {
        let a = KmvSketch::new(1024);
        let b = KmvSketch::new(64);
        assert_eq!(a.merge(&b).k(), 64);
    }

    #[test]
    fn self_merge_is_idempotent() {
        let a = sketch_of(128, (0..10_000).map(|i| format!("x{i}")));
        assert_eq!(a.merge(&a), a);
    }

    #[test]
    fn jaccard_of_half_overlapping_sets() {
        // A = 0..20k, B = 10k..30k: |A∩B|=10k, |A∪B|=30k, J = 1/3.
        let a = sketch_of(1024, (0..20_000).map(|i| format!("u{i}")));
        let b = sketch_of(1024, (10_000..30_000).map(|i| format!("u{i}")));
        let j = a.jaccard(&b);
        assert!((j - 1.0 / 3.0).abs() < 0.08, "jaccard {j} not near 1/3");
        // Disjoint sets estimate near zero.
        let c = sketch_of(1024, (0..20_000).map(|i| format!("v{i}")));
        assert!(a.jaccard(&c) < 0.05);
        // Identical sets estimate 1.
        assert!((a.jaccard(&a) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn bytes_round_trip() {
        let s = sketch_of(512, (0..2_000).map(|i| format!("r{i}")));
        let bytes = s.to_bytes();
        assert_eq!(KmvSketch::from_bytes(&bytes).unwrap(), s);
        // Empty sketch round-trips too.
        let e = KmvSketch::new(1024);
        assert_eq!(KmvSketch::from_bytes(&e.to_bytes()).unwrap(), e);
    }

    #[test]
    fn from_bytes_rejects_corruption() {
        let s = sketch_of(64, (0..100).map(|i| format!("c{i}")));
        let bytes = s.to_bytes();
        assert!(KmvSketch::from_bytes(&bytes[..bytes.len() - 3]).is_err());
        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'X';
        assert!(KmvSketch::from_bytes(&bad_magic).is_err());
        let mut bad_version = bytes.clone();
        bad_version[3] = 9;
        assert!(KmvSketch::from_bytes(&bad_version).is_err());
        let mut unsorted = bytes.clone();
        unsorted[HEADER_LEN..HEADER_LEN + 16].rotate_left(8); // swap first two hashes
        assert!(KmvSketch::from_bytes(&unsorted).is_err());
    }

    /// Golden format fixture: locks byte-format v1 (including the hash
    /// function). If this test breaks, you changed the persisted format —
    /// bump VERSION and add a legacy decode path instead.
    #[test]
    fn golden_format_v1() {
        let mut s = KmvSketch::new(8);
        for item in ["alpha", "beta", "gamma"] {
            s.insert(item.as_bytes());
        }
        let hex: String = s.to_bytes().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "4b4d56010800000003000000f6299d6fbff7700041f6df977ffffa285aab25f6b50369be",
        );
    }
}
