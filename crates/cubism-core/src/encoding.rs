//! Compact binary XUnit keys.
//!
//! The legacy library's dominant inefficiency was passing XUnits as strings
//! and re-parsing them everywhere. Here the group-by key is a compact binary
//! encoding against a per-cube string dictionary; the string form exists only
//! at parse/present boundaries.
//!
//! Layout (all integers big-endian, so byte order is stable):
//!
//! ```text
//! key      := ypath*                      (empty key = global rollup /G)
//! ypath    := dim_id:u32  n_attrs:u8  attr*
//! attr     := name_id:u32 value_id:u32   (value_id NULL_ID encodes None)
//! ```
//!
//! Keys are encoded from *normalized* XUnits, so equal cells produce
//! byte-identical keys against the same dictionary. The dictionary is
//! append-only and emitted alongside cube output as a sidecar table.

use crate::error::CubismError;
use crate::ypath::{XUnit, YPath};
use std::collections::HashMap;

/// Sentinel value-id encoding a `None` attribute value.
pub const NULL_ID: u32 = u32::MAX;

const YPATH_HEADER: usize = 5; // dim_id:u32 + n_attrs:u8
const ATTR_LEN: usize = 8; // name_id:u32 + value_id:u32

/// Append-only string interner shared by dimension names, attribute names,
/// and attribute values within one cube build.
#[derive(Debug, Default, Clone)]
pub struct XUnitDictionary {
    strings: Vec<String>,
    index: HashMap<String, u32>,
}

impl XUnitDictionary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.index.get(s) {
            return id;
        }
        let id = u32::try_from(self.strings.len()).expect("dictionary overflow");
        assert!(id != NULL_ID, "dictionary exhausted u32 id space");
        self.strings.push(s.to_string());
        self.index.insert(s.to_string(), id);
        id
    }

    pub fn get(&self, s: &str) -> Option<u32> {
        self.index.get(s).copied()
    }

    pub fn lookup(&self, id: u32) -> Option<&str> {
        self.strings.get(id as usize).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.strings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }

    /// All interned strings in id order (for emitting the sidecar table).
    pub fn strings(&self) -> &[String] {
        &self.strings
    }
}

/// Encode a (normalized) XUnit as a binary key, interning new strings.
pub fn encode_xunit(xunit: &XUnit, dict: &mut XUnitDictionary) -> Vec<u8> {
    let normalized = xunit.clone().normalize();
    let size = normalized
        .ypaths
        .iter()
        .map(|yp| YPATH_HEADER + yp.attributes.len() * ATTR_LEN)
        .sum();
    let mut out = Vec::with_capacity(size);
    for yp in &normalized.ypaths {
        assert!(yp.attributes.len() <= u8::MAX as usize, "YPath deeper than 255 levels");
        out.extend_from_slice(&dict.intern(&yp.dim).to_be_bytes());
        out.push(yp.attributes.len() as u8);
        for (name, value) in &yp.attributes {
            out.extend_from_slice(&dict.intern(name).to_be_bytes());
            let value_id = match value {
                Some(v) => dict.intern(v),
                None => NULL_ID,
            };
            out.extend_from_slice(&value_id.to_be_bytes());
        }
    }
    out
}

/// Decode a binary key back into an XUnit using a read-only dictionary.
pub fn decode_xunit(bytes: &[u8], dict: &XUnitDictionary) -> Result<XUnit, CubismError> {
    let mut ypaths = Vec::new();
    let mut cursor = bytes;

    let take_u32 = |cursor: &mut &[u8]| -> Result<u32, CubismError> {
        let (head, rest) = cursor
            .split_first_chunk::<4>()
            .ok_or_else(|| CubismError::Decode("truncated key".into()))?;
        *cursor = rest;
        Ok(u32::from_be_bytes(*head))
    };
    let resolve = |id: u32| -> Result<String, CubismError> {
        dict.lookup(id)
            .map(String::from)
            .ok_or_else(|| CubismError::Decode(format!("id {id} not in dictionary")))
    };

    while !cursor.is_empty() {
        let dim = resolve(take_u32(&mut cursor)?)?;
        let (&n_attrs, rest) = cursor
            .split_first()
            .ok_or_else(|| CubismError::Decode("truncated key at attribute count".into()))?;
        cursor = rest;

        let mut attributes = Vec::with_capacity(n_attrs as usize);
        for _ in 0..n_attrs {
            let name = resolve(take_u32(&mut cursor)?)?;
            let value_id = take_u32(&mut cursor)?;
            let value = if value_id == NULL_ID { None } else { Some(resolve(value_id)?) };
            attributes.push((name, value));
        }
        ypaths.push(YPath { dim, attributes });
    }

    Ok(XUnit { ypaths })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> XUnit {
        XUnit::new(vec![
            YPath::new("geo").with_attribute("country", "CZ").with_attribute("city", "Prague"),
            YPath::new("gender").with_attribute("gender", "F"),
        ])
    }

    #[test]
    fn encode_decode_round_trip() {
        let mut dict = XUnitDictionary::new();
        let key = encode_xunit(&sample(), &mut dict);
        let decoded = decode_xunit(&key, &dict).unwrap();
        assert_eq!(decoded, sample().normalize());
    }

    #[test]
    fn equal_cells_produce_identical_keys_regardless_of_order() {
        let mut dict = XUnitDictionary::new();
        let a = sample();
        let mut reversed = sample();
        reversed.ypaths.reverse();
        assert_eq!(encode_xunit(&a, &mut dict), encode_xunit(&reversed, &mut dict));
    }

    #[test]
    fn global_is_the_empty_key() {
        let mut dict = XUnitDictionary::new();
        let key = encode_xunit(&XUnit::global(), &mut dict);
        assert!(key.is_empty());
        assert!(decode_xunit(&key, &dict).unwrap().is_global());
    }

    #[test]
    fn none_value_round_trips() {
        let x = XUnit::new(vec![YPath {
            dim: "geo".into(),
            attributes: vec![("country".into(), None)],
        }]);
        let mut dict = XUnitDictionary::new();
        let key = encode_xunit(&x, &mut dict);
        assert_eq!(decode_xunit(&key, &dict).unwrap(), x);
    }

    #[test]
    fn dictionary_reuses_ids() {
        let mut dict = XUnitDictionary::new();
        encode_xunit(&sample(), &mut dict);
        let before = dict.len();
        encode_xunit(&sample(), &mut dict);
        assert_eq!(dict.len(), before);
    }

    #[test]
    fn decode_rejects_truncated_and_unknown() {
        let mut dict = XUnitDictionary::new();
        let key = encode_xunit(&sample(), &mut dict);
        assert!(decode_xunit(&key[..key.len() - 1], &dict).is_err());
        assert!(decode_xunit(&key, &XUnitDictionary::new()).is_err());
    }
}
