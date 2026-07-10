//! Dense embedding centroid: per cube cell, the running mean of a
//! fixed-dimension float vector (document embeddings, feature vectors).
//!
//! The buffer stores (count, element-wise sum), so merge is element-wise
//! addition — associative up to float rounding. Presenting divides by
//! count. Semantic-search-over-cells builds on this: each cell's centroid
//! is its retrieval embedding.

use crate::error::CubismError;

const MAGIC: &[u8; 3] = b"CTR";
const VERSION: u8 = 1;
const HEADER_LEN: usize = 3 + 1 + 8 + 4; // magic + version + count + dim

#[derive(Debug, Clone, PartialEq)]
pub struct Centroid {
    count: u64,
    /// Element-wise sums; empty until the first vector fixes the dimension.
    sums: Vec<f64>,
}

impl Default for Centroid {
    fn default() -> Self {
        Self::new()
    }
}

impl Centroid {
    pub fn new() -> Self {
        Centroid { count: 0, sums: Vec::new() }
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    pub fn dim(&self) -> usize {
        self.sums.len()
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn add(&mut self, vector: &[f64]) -> Result<(), CubismError> {
        if self.count == 0 {
            self.sums = vector.to_vec();
            self.count = 1;
            return Ok(());
        }
        if vector.len() != self.sums.len() {
            return Err(CubismError::Decode(format!(
                "centroid dimension mismatch: expected {}, got {}",
                self.sums.len(),
                vector.len()
            )));
        }
        for (s, v) in self.sums.iter_mut().zip(vector) {
            *s += v;
        }
        self.count += 1;
        Ok(())
    }

    pub fn merge(&self, other: &Centroid) -> Result<Centroid, CubismError> {
        if other.is_empty() {
            return Ok(self.clone());
        }
        if self.is_empty() {
            return Ok(other.clone());
        }
        if self.dim() != other.dim() {
            return Err(CubismError::Decode(format!(
                "centroid dimension mismatch on merge: {} vs {}",
                self.dim(),
                other.dim()
            )));
        }
        Ok(Centroid {
            count: self.count + other.count,
            sums: self.sums.iter().zip(&other.sums).map(|(a, b)| a + b).collect(),
        })
    }

    /// The mean vector; `None` while empty.
    pub fn mean(&self) -> Option<Vec<f64>> {
        if self.count == 0 {
            None
        } else {
            Some(self.sums.iter().map(|s| s / self.count as f64).collect())
        }
    }

    /// Format v1: `"CTR" ver:u8 count:u64 dim:u32 sum:f64*dim`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.sums.len() * 8);
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&(self.sums.len() as u32).to_le_bytes());
        for s in &self.sums {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CubismError> {
        let err = |reason: &str| CubismError::Decode(format!("Centroid: {reason}"));
        if bytes.len() < HEADER_LEN || &bytes[0..3] != MAGIC {
            return Err(err("bad magic or truncated header"));
        }
        if bytes[3] != VERSION {
            return Err(err("unsupported version"));
        }
        let count = u64::from_le_bytes(bytes[4..12].try_into().unwrap());
        let dim = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        if bytes.len() != HEADER_LEN + dim * 8 {
            return Err(err("length does not match dimension"));
        }
        let sums = bytes[HEADER_LEN..]
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        Ok(Centroid { count, sums })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_of_added_vectors() {
        let mut c = Centroid::new();
        c.add(&[1.0, 2.0]).unwrap();
        c.add(&[3.0, 4.0]).unwrap();
        assert_eq!(c.mean(), Some(vec![2.0, 3.0]));
        assert_eq!(c.count(), 2);
    }

    #[test]
    fn dimension_mismatch_is_an_error() {
        let mut c = Centroid::new();
        c.add(&[1.0, 2.0]).unwrap();
        assert!(c.add(&[1.0]).is_err());
    }

    #[test]
    fn merge_matches_sequential_adds() {
        let mut a = Centroid::new();
        a.add(&[1.0, 0.0]).unwrap();
        let mut b = Centroid::new();
        b.add(&[0.0, 1.0]).unwrap();
        b.add(&[2.0, 3.0]).unwrap();
        let m = a.merge(&b).unwrap();
        assert_eq!(m.count(), 3);
        assert_eq!(m.mean(), Some(vec![1.0, 4.0 / 3.0]));
        // Empty merges are identity.
        assert_eq!(a.merge(&Centroid::new()).unwrap(), a);
        assert_eq!(Centroid::new().merge(&a).unwrap(), a);
    }

    #[test]
    fn bytes_round_trip() {
        let mut c = Centroid::new();
        c.add(&[0.5, -1.5, 3.25]).unwrap();
        c.add(&[1.5, 2.5, -0.25]).unwrap();
        assert_eq!(Centroid::from_bytes(&c.to_bytes()).unwrap(), c);
        let e = Centroid::new();
        assert_eq!(Centroid::from_bytes(&e.to_bytes()).unwrap(), e);
    }

    #[test]
    fn golden_format_v1() {
        let mut c = Centroid::new();
        c.add(&[1.0, -2.0]).unwrap();
        let hex: String = c.to_bytes().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "435452010100000000000000\
             02000000000000000000f03f00000000000000c0",
        );
    }
}
