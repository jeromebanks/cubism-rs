//! Bounded top-k by summed score ("which tools cost the most tokens",
//! "top pages by views").
//!
//! A keyed heavy-hitters structure: scores are summed per key, and when the
//! tracked set outgrows `capacity * OVERFLOW_FACTOR` the lowest-scored keys
//! are pruned. Exact while distinct keys fit in the tracked set; beyond
//! that, long-tail keys that would only become significant later can be
//! undercounted (the standard bounded-top-k trade-off — like any
//! frequent-items sketch, merge results can depend on merge order once
//! pruning kicks in).
//!
//! Diverges from the legacy `ArgMaxMap`, which kept a score-sorted list
//! *without* key dedup and relied on upstream pre-aggregation; summing per
//! key here makes `top_k` usable directly as a measure.

use crate::error::CubismError;
use std::collections::HashMap;

pub const DEFAULT_TOPK_CAPACITY: u32 = 100;
/// Track this many times `capacity` before pruning, to keep tail error low.
const OVERFLOW_FACTOR: usize = 4;

const MAGIC: &[u8; 3] = b"TPK";
const VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct TopK {
    capacity: u32,
    entries: HashMap<String, f64>,
}

impl TopK {
    pub fn new(capacity: u32) -> Self {
        assert!(capacity >= 1, "top-k capacity must be at least 1");
        TopK {
            capacity,
            entries: HashMap::new(),
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

    pub fn add(&mut self, key: &str, score: f64) {
        match self.entries.get_mut(key) {
            Some(s) => *s += score,
            None => {
                self.entries.insert(key.to_string(), score);
                self.prune();
            }
        }
    }

    pub fn merge(&self, other: &TopK) -> TopK {
        let capacity = self.capacity.min(other.capacity);
        let mut entries = self.entries.clone();
        for (key, score) in &other.entries {
            *entries.entry(key.clone()).or_insert(0.0) += score;
        }
        let mut merged = TopK { capacity, entries };
        merged.prune();
        merged
    }

    fn max_tracked(&self) -> usize {
        self.capacity as usize * OVERFLOW_FACTOR
    }

    fn prune(&mut self) {
        if self.entries.len() <= self.max_tracked() {
            return;
        }
        // Cut back to `capacity`, not `max_tracked`: pruning to the
        // threshold would evict one entry per new key, degenerating every
        // add into an O(n log n) re-sort. Pruning to `capacity` restores
        // the overflow headroom, so re-sorts amortize over
        // `capacity * (OVERFLOW_FACTOR - 1)` inserts.
        let mut ranked = self.ranked();
        ranked.truncate(self.capacity as usize);
        self.entries = ranked.into_iter().collect();
    }

    /// All tracked entries, best-first (score desc, key asc as tiebreak —
    /// the deterministic order used everywhere, including serialization).
    fn ranked(&self) -> Vec<(String, f64)> {
        let mut v: Vec<(String, f64)> = self.entries.iter().map(|(k, s)| (k.clone(), *s)).collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v
    }

    /// The top `capacity` entries, best-first.
    pub fn top(&self) -> Vec<(String, f64)> {
        let mut ranked = self.ranked();
        ranked.truncate(self.capacity as usize);
        ranked
    }

    /// Present as JSON: `[{"key":"a","score":3.0},...]`.
    pub fn to_json(&self) -> String {
        let items: Vec<serde_json::Value> = self
            .top()
            .into_iter()
            .map(|(key, score)| serde_json::json!({"key": key, "score": score}))
            .collect();
        serde_json::Value::Array(items).to_string()
    }

    /// Format v1: `"TPK" ver:u8 capacity:u32 n:u32 (score:f64 len:u16 utf8)*n`,
    /// entries best-first.
    pub fn to_bytes(&self) -> Vec<u8> {
        let ranked = self.ranked();
        let mut out = Vec::with_capacity(12 + ranked.len() * 16);
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&self.capacity.to_le_bytes());
        out.extend_from_slice(&(ranked.len() as u32).to_le_bytes());
        for (key, score) in &ranked {
            out.extend_from_slice(&score.to_le_bytes());
            let bytes = key.as_bytes();
            out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
            out.extend_from_slice(bytes);
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CubismError> {
        let err = |reason: &str| CubismError::Decode(format!("TopK sketch: {reason}"));
        if bytes.len() < 12 || &bytes[0..3] != MAGIC {
            return Err(err("bad magic or truncated header"));
        }
        if bytes[3] != VERSION {
            return Err(err("unsupported version"));
        }
        let capacity = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let n = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let mut entries = HashMap::with_capacity(n);
        let mut cursor = &bytes[12..];
        for _ in 0..n {
            if cursor.len() < 10 {
                return Err(err("truncated entry"));
            }
            let score = f64::from_le_bytes(cursor[0..8].try_into().unwrap());
            let len = u16::from_le_bytes(cursor[8..10].try_into().unwrap()) as usize;
            if cursor.len() < 10 + len {
                return Err(err("truncated key"));
            }
            let key =
                std::str::from_utf8(&cursor[10..10 + len]).map_err(|_| err("key is not utf8"))?;
            entries.insert(key.to_string(), score);
            cursor = &cursor[10 + len..];
        }
        if !cursor.is_empty() {
            return Err(err("trailing bytes"));
        }
        Ok(TopK { capacity, entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_scores_per_key_and_ranks() {
        let mut t = TopK::new(2);
        t.add("bash", 10.0);
        t.add("read", 5.0);
        t.add("bash", 7.0);
        t.add("edit", 1.0);
        assert_eq!(t.top(), vec![("bash".into(), 17.0), ("read".into(), 5.0)]);
    }

    #[test]
    fn merge_sums_across_sketches() {
        let mut a = TopK::new(4);
        let mut b = TopK::new(4);
        a.add("x", 3.0);
        a.add("y", 1.0);
        b.add("x", 2.0);
        b.add("z", 4.0);
        let m = a.merge(&b);
        assert_eq!(m.top()[0], ("x".into(), 5.0));
        assert_eq!(m.top()[1], ("z".into(), 4.0));
    }

    #[test]
    fn prunes_but_keeps_overflow_headroom() {
        let mut t = TopK::new(2);
        for i in 0..100 {
            t.add(&format!("k{i}"), i as f64);
        }
        assert!(t.len() <= 2 * OVERFLOW_FACTOR);
        assert_eq!(t.top(), vec![("k99".into(), 99.0), ("k98".into(), 98.0)]);
    }

    #[test]
    fn bytes_round_trip() {
        let mut t = TopK::new(8);
        for (k, s) in [("a", 1.5), ("b", -2.0), ("c", 0.0)] {
            t.add(k, s);
        }
        assert_eq!(TopK::from_bytes(&t.to_bytes()).unwrap(), t);
        let e = TopK::new(4);
        assert_eq!(TopK::from_bytes(&e.to_bytes()).unwrap(), e);
    }

    #[test]
    fn json_presentation() {
        let mut t = TopK::new(2);
        t.add("bash", 2.0);
        t.add("read", 1.0);
        assert_eq!(
            t.to_json(),
            r#"[{"key":"bash","score":2.0},{"key":"read","score":1.0}]"#
        );
    }

    #[test]
    fn golden_format_v1() {
        let mut t = TopK::new(4);
        t.add("beta", 2.5);
        t.add("alpha", 1.0);
        let hex: String = t.to_bytes().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "54504b010400000002000000\
             0000000000000440040062657461\
             000000000000f03f0500616c706861",
        );
    }
}
