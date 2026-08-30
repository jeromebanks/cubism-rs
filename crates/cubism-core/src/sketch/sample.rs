//! Bounded exemplar sample: a uniform sample of **distinct** values per
//! cube cell ("give me ~64 representative doc IDs from this slice"), the
//! retrieval-unit building block for RAG-over-cubes.
//!
//! Implemented as bottom-k-by-hash rather than a classic randomized
//! reservoir: keep the `capacity` values whose xxh3 hashes are smallest.
//! Because the hash is a deterministic function of the value, the sample is
//! uniform over distinct values, insertion-order independent, and the merge
//! is exactly the KMV sorted merge — **fully associative and commutative**,
//! which a randomized reservoir merge is not.

use crate::error::CubismError;
use crate::sketch::kmv::hash_value;

pub const DEFAULT_SAMPLE_CAPACITY: u32 = 64;

const MAGIC: &[u8; 3] = b"SMP";
const VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct ExemplarSample {
    capacity: u32,
    /// Sorted ascending by hash, distinct hashes, `len <= capacity`.
    entries: Vec<(u64, String)>,
}

impl ExemplarSample {
    pub fn new(capacity: u32) -> Self {
        assert!(capacity >= 1, "sample capacity must be at least 1");
        ExemplarSample {
            capacity,
            entries: Vec::new(),
        }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn insert(&mut self, value: &str) {
        let hash = hash_value(value.as_bytes());
        match self.entries.binary_search_by_key(&hash, |(h, _)| *h) {
            Ok(_) => {} // distinct sample: value (well, its hash) already present
            Err(pos) => {
                if pos < self.capacity as usize {
                    self.entries.insert(pos, (hash, value.to_string()));
                    self.entries.truncate(self.capacity as usize);
                }
            }
        }
    }

    pub fn merge(&self, other: &ExemplarSample) -> ExemplarSample {
        let capacity = self.capacity.min(other.capacity);
        let mut merged = Vec::with_capacity((self.len() + other.len()).min(capacity as usize));
        let (mut i, mut j) = (0, 0);
        while merged.len() < capacity as usize {
            match (self.entries.get(i), other.entries.get(j)) {
                (Some(a), Some(b)) => {
                    if a.0 < b.0 {
                        merged.push(a.clone());
                        i += 1;
                    } else if b.0 < a.0 {
                        merged.push(b.clone());
                        j += 1;
                    } else {
                        merged.push(a.clone());
                        i += 1;
                        j += 1;
                    }
                }
                (Some(a), None) => {
                    merged.push(a.clone());
                    i += 1;
                }
                (None, Some(b)) => {
                    merged.push(b.clone());
                    j += 1;
                }
                (None, None) => break,
            }
        }
        ExemplarSample {
            capacity,
            entries: merged,
        }
    }

    /// The sampled values (hash order — effectively random order).
    pub fn values(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(_, v)| v.as_str())
    }

    /// Present as a JSON array of the sampled values.
    pub fn to_json(&self) -> String {
        serde_json::Value::Array(
            self.values()
                .map(|v| serde_json::Value::String(v.to_string()))
                .collect(),
        )
        .to_string()
    }

    /// Format v1: `"SMP" ver:u8 capacity:u32 n:u32 (hash:u64 len:u16 utf8)*n`,
    /// hash-ascending.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + self.entries.len() * 16);
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&self.capacity.to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (hash, value) in &self.entries {
            out.extend_from_slice(&hash.to_le_bytes());
            let bytes = value.as_bytes();
            out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
            out.extend_from_slice(bytes);
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CubismError> {
        let err = |reason: &str| CubismError::Decode(format!("ExemplarSample: {reason}"));
        if bytes.len() < 12 || &bytes[0..3] != MAGIC {
            return Err(err("bad magic or truncated header"));
        }
        if bytes[3] != VERSION {
            return Err(err("unsupported version"));
        }
        let capacity = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let n = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let mut entries = Vec::with_capacity(n);
        let mut cursor = &bytes[12..];
        for _ in 0..n {
            if cursor.len() < 10 {
                return Err(err("truncated entry"));
            }
            let hash = u64::from_le_bytes(cursor[0..8].try_into().unwrap());
            let len = u16::from_le_bytes(cursor[8..10].try_into().unwrap()) as usize;
            if cursor.len() < 10 + len {
                return Err(err("truncated value"));
            }
            let value =
                std::str::from_utf8(&cursor[10..10 + len]).map_err(|_| err("value is not utf8"))?;
            entries.push((hash, value.to_string()));
            cursor = &cursor[10 + len..];
        }
        if !cursor.is_empty() {
            return Err(err("trailing bytes"));
        }
        if !entries.is_sorted_by_key(|(h, _)| *h) || entries.windows(2).any(|w| w[0].0 == w[1].0) {
            return Err(err("entries are not sorted-distinct by hash"));
        }
        Ok(ExemplarSample { capacity, entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_of(capacity: u32, items: impl IntoIterator<Item = String>) -> ExemplarSample {
        let mut s = ExemplarSample::new(capacity);
        for item in items {
            s.insert(&item);
        }
        s
    }

    #[test]
    fn keeps_at_most_capacity_distinct_values() {
        let s = sample_of(16, (0..1000).map(|i| format!("doc{i}")));
        assert_eq!(s.len(), 16);
        let s2 = sample_of(16, (0..10).map(|i| format!("doc{i}")));
        assert_eq!(s2.len(), 10);
    }

    #[test]
    fn deterministic_and_order_independent() {
        let fwd = sample_of(8, (0..500).map(|i| format!("d{i}")));
        let rev = sample_of(8, (0..500).rev().map(|i| format!("d{i}")));
        assert_eq!(fwd, rev);
    }

    #[test]
    fn merge_equals_union_build() {
        let a = sample_of(8, (0..300).map(|i| format!("a{i}")));
        let b = sample_of(8, (0..300).map(|i| format!("b{i}")));
        let direct = sample_of(8, (0..300).flat_map(|i| [format!("a{i}"), format!("b{i}")]));
        assert_eq!(a.merge(&b), direct);
        assert_eq!(a.merge(&b), b.merge(&a));
    }

    #[test]
    fn bytes_round_trip() {
        let s = sample_of(8, (0..100).map(|i| format!("v{i}")));
        assert_eq!(ExemplarSample::from_bytes(&s.to_bytes()).unwrap(), s);
    }

    #[test]
    fn golden_format_v1() {
        let mut s = ExemplarSample::new(4);
        s.insert("alpha");
        s.insert("beta");
        let hex: String = s.to_bytes().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "534d5001040000000200000041f6df977ffffa280400626574615aab25f6b50369be0500616c706861",
        );
    }
}
