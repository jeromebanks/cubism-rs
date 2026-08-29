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
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Sentinel value-id encoding a `None` attribute value.
pub const NULL_ID: u32 = u32::MAX;

const YPATH_HEADER: usize = 5; // dim_id:u32 + n_attrs:u8
const ATTR_LEN: usize = 8; // name_id:u32 + value_id:u32

const CANONICAL_MAGIC: &[u8; 3] = b"CXU";
const CANONICAL_VERSION: u8 = 1;

/// Dictionary-independent typed value used by the persisted canonical XUnit
/// identity. Static v1 artifacts continue to use [`encode_xunit`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CanonicalValue {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(String),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalAttribute {
    pub name: String,
    pub value: CanonicalValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalYPath {
    pub dim: String,
    /// Hierarchy order is semantic and is therefore preserved.
    pub attributes: Vec<CanonicalAttribute>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalXUnit {
    pub ypaths: Vec<CanonicalYPath>,
}

impl CanonicalXUnit {
    pub fn global() -> Self {
        Self { ypaths: Vec::new() }
    }

    pub fn normalize(mut self) -> Self {
        self.ypaths.sort_by(|left, right| left.dim.cmp(&right.dim));
        self
    }
}

impl From<&XUnit> for CanonicalXUnit {
    fn from(xunit: &XUnit) -> Self {
        Self {
            ypaths: xunit
                .ypaths
                .iter()
                .map(|ypath| CanonicalYPath {
                    dim: ypath.dim.clone(),
                    attributes: ypath
                        .attributes
                        .iter()
                        .map(|(name, value)| CanonicalAttribute {
                            name: name.clone(),
                            value: value.as_ref().map_or(CanonicalValue::Null, |value| {
                                CanonicalValue::String(value.clone())
                            }),
                        })
                        .collect(),
                })
                .collect(),
        }
        .normalize()
    }
}

/// Stable BLAKE3-256 content ID of canonical XUnit bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct XUnitContentId([u8; 32]);

impl XUnitContentId {
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

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
        assert!(
            yp.attributes.len() <= u8::MAX as usize,
            "YPath deeper than 255 levels"
        );
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
            let value = if value_id == NULL_ID {
                None
            } else {
                Some(resolve(value_id)?)
            };
            attributes.push((name, value));
        }
        ypaths.push(YPath { dim, attributes });
    }

    Ok(XUnit { ypaths })
}

/// Encode a typed XUnit without dictionary-local IDs.
///
/// Layout:
///
/// ```text
/// "CXU" version:u8 ypath_count:u32
///   (utf8(dim) attr_count:u32 (utf8(name) typed_value)*)*
/// ```
///
/// Strings and byte arrays are length-prefixed with big-endian `u32`.
/// Integer and finite IEEE-754 payloads are big-endian. YPaths are sorted by
/// dimension; duplicate dimensions are rejected.
pub fn encode_canonical_xunit(xunit: &CanonicalXUnit) -> Result<Vec<u8>, CubismError> {
    let normalized = xunit.clone().normalize();
    if normalized
        .ypaths
        .windows(2)
        .any(|pair| pair[0].dim == pair[1].dim)
    {
        return Err(CubismError::CanonicalEncoding(
            "an XUnit may contain at most one YPath per dimension".into(),
        ));
    }

    let mut out = Vec::new();
    out.extend_from_slice(CANONICAL_MAGIC);
    out.push(CANONICAL_VERSION);
    push_u32(
        &mut out,
        normalized.ypaths.len(),
        "canonical XUnit YPath count",
    )?;
    for ypath in &normalized.ypaths {
        push_bytes(&mut out, ypath.dim.as_bytes(), "dimension name")?;
        push_u32(
            &mut out,
            ypath.attributes.len(),
            "canonical YPath attribute count",
        )?;
        for attribute in &ypath.attributes {
            push_bytes(&mut out, attribute.name.as_bytes(), "attribute name")?;
            encode_canonical_value(&mut out, &attribute.value)?;
        }
    }
    Ok(out)
}

pub fn decode_canonical_xunit(bytes: &[u8]) -> Result<CanonicalXUnit, CubismError> {
    let mut cursor = bytes;
    if take_exact(&mut cursor, 3)? != CANONICAL_MAGIC {
        return Err(CubismError::CanonicalEncoding(
            "canonical XUnit has bad magic".into(),
        ));
    }
    let version = take_exact(&mut cursor, 1)?[0];
    if version != CANONICAL_VERSION {
        return Err(CubismError::CanonicalEncoding(format!(
            "unsupported canonical XUnit version {version}"
        )));
    }
    let ypath_count = take_u32(&mut cursor)? as usize;
    let mut ypaths = Vec::with_capacity(ypath_count.min(1024));
    let mut previous_dim: Option<String> = None;
    for _ in 0..ypath_count {
        let dim = take_string(&mut cursor, "dimension name")?;
        if previous_dim
            .as_ref()
            .is_some_and(|previous| previous >= &dim)
        {
            return Err(CubismError::CanonicalEncoding(
                "canonical XUnit dimensions are not strictly sorted".into(),
            ));
        }
        previous_dim = Some(dim.clone());
        let attribute_count = take_u32(&mut cursor)? as usize;
        let mut attributes = Vec::with_capacity(attribute_count.min(256));
        for _ in 0..attribute_count {
            let name = take_string(&mut cursor, "attribute name")?;
            let value = decode_canonical_value(&mut cursor)?;
            attributes.push(CanonicalAttribute { name, value });
        }
        ypaths.push(CanonicalYPath { dim, attributes });
    }
    if !cursor.is_empty() {
        return Err(CubismError::CanonicalEncoding(
            "canonical XUnit has trailing bytes".into(),
        ));
    }
    Ok(CanonicalXUnit { ypaths })
}

pub fn canonical_xunit_content_id(xunit: &CanonicalXUnit) -> Result<XUnitContentId, CubismError> {
    let bytes = encode_canonical_xunit(xunit)?;
    Ok(XUnitContentId(*blake3::hash(&bytes).as_bytes()))
}

fn push_u32(out: &mut Vec<u8>, value: usize, field: &str) -> Result<(), CubismError> {
    let value = u32::try_from(value)
        .map_err(|_| CubismError::CanonicalEncoding(format!("{field} exceeds u32")))?;
    out.extend_from_slice(&value.to_be_bytes());
    Ok(())
}

fn push_bytes(out: &mut Vec<u8>, bytes: &[u8], field: &str) -> Result<(), CubismError> {
    push_u32(out, bytes.len(), field)?;
    out.extend_from_slice(bytes);
    Ok(())
}

fn encode_canonical_value(out: &mut Vec<u8>, value: &CanonicalValue) -> Result<(), CubismError> {
    match value {
        CanonicalValue::Null => out.push(0),
        CanonicalValue::Bool(false) => out.push(1),
        CanonicalValue::Bool(true) => out.push(2),
        CanonicalValue::I64(value) => {
            out.push(3);
            out.extend_from_slice(&value.to_be_bytes());
        }
        CanonicalValue::U64(value) => {
            out.push(4);
            out.extend_from_slice(&value.to_be_bytes());
        }
        CanonicalValue::F64(value) => {
            if !value.is_finite() {
                return Err(CubismError::CanonicalEncoding(
                    "canonical floating-point values must be finite".into(),
                ));
            }
            out.push(5);
            let canonical = if *value == 0.0 { 0.0 } else { *value };
            out.extend_from_slice(&canonical.to_bits().to_be_bytes());
        }
        CanonicalValue::String(value) => {
            out.push(6);
            push_bytes(out, value.as_bytes(), "string value")?;
        }
        CanonicalValue::Bytes(value) => {
            out.push(7);
            push_bytes(out, value, "byte value")?;
        }
    }
    Ok(())
}

fn decode_canonical_value(cursor: &mut &[u8]) -> Result<CanonicalValue, CubismError> {
    let tag = take_exact(cursor, 1)?[0];
    match tag {
        0 => Ok(CanonicalValue::Null),
        1 => Ok(CanonicalValue::Bool(false)),
        2 => Ok(CanonicalValue::Bool(true)),
        3 => Ok(CanonicalValue::I64(i64::from_be_bytes(
            take_exact(cursor, 8)?.try_into().unwrap(),
        ))),
        4 => Ok(CanonicalValue::U64(u64::from_be_bytes(
            take_exact(cursor, 8)?.try_into().unwrap(),
        ))),
        5 => {
            let value = f64::from_bits(u64::from_be_bytes(
                take_exact(cursor, 8)?.try_into().unwrap(),
            ));
            if !value.is_finite() || (value == 0.0 && value.is_sign_negative()) {
                return Err(CubismError::CanonicalEncoding(
                    "canonical floating-point payload is non-finite or negative zero".into(),
                ));
            }
            Ok(CanonicalValue::F64(value))
        }
        6 => Ok(CanonicalValue::String(take_string(cursor, "string value")?)),
        7 => {
            let length = take_u32(cursor)? as usize;
            Ok(CanonicalValue::Bytes(take_exact(cursor, length)?.to_vec()))
        }
        _ => Err(CubismError::CanonicalEncoding(format!(
            "unknown canonical value tag {tag}"
        ))),
    }
}

fn take_exact<'a>(cursor: &mut &'a [u8], length: usize) -> Result<&'a [u8], CubismError> {
    if cursor.len() < length {
        return Err(CubismError::CanonicalEncoding(
            "truncated canonical XUnit".into(),
        ));
    }
    let (head, tail) = cursor.split_at(length);
    *cursor = tail;
    Ok(head)
}

fn take_u32(cursor: &mut &[u8]) -> Result<u32, CubismError> {
    Ok(u32::from_be_bytes(
        take_exact(cursor, 4)?.try_into().unwrap(),
    ))
}

fn take_string(cursor: &mut &[u8], field: &str) -> Result<String, CubismError> {
    let length = take_u32(cursor)? as usize;
    let bytes = take_exact(cursor, length)?;
    std::str::from_utf8(bytes)
        .map(String::from)
        .map_err(|_| CubismError::CanonicalEncoding(format!("{field} is not UTF-8")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> XUnit {
        XUnit::new(vec![
            YPath::new("geo")
                .with_attribute("country", "CZ")
                .with_attribute("city", "Prague"),
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
        assert_eq!(
            encode_xunit(&a, &mut dict),
            encode_xunit(&reversed, &mut dict)
        );
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

    #[test]
    fn canonical_round_trip_supports_typed_unicode_and_sentinel_values() {
        let xunit = CanonicalXUnit {
            ypaths: vec![
                CanonicalYPath {
                    dim: "typed".into(),
                    attributes: vec![
                        CanonicalAttribute {
                            name: "null".into(),
                            value: CanonicalValue::Null,
                        },
                        CanonicalAttribute {
                            name: "bool".into(),
                            value: CanonicalValue::Bool(true),
                        },
                        CanonicalAttribute {
                            name: "int".into(),
                            value: CanonicalValue::I64(-42),
                        },
                        CanonicalAttribute {
                            name: "float".into(),
                            value: CanonicalValue::F64(1.5),
                        },
                        CanonicalAttribute {
                            name: "text".into(),
                            value: CanonicalValue::String("雪/___/###/+++".into()),
                        },
                    ],
                },
                CanonicalYPath {
                    dim: "bytes".into(),
                    attributes: vec![CanonicalAttribute {
                        name: "payload".into(),
                        value: CanonicalValue::Bytes(vec![0, 255, b'/']),
                    }],
                },
            ],
        };
        let bytes = encode_canonical_xunit(&xunit).unwrap();
        assert_eq!(
            decode_canonical_xunit(&bytes).unwrap(),
            xunit.clone().normalize()
        );
    }

    #[test]
    fn canonical_identity_is_dictionary_and_order_independent() {
        let xunit = sample();
        let mut reversed = xunit.clone();
        reversed.ypaths.reverse();
        let canonical = CanonicalXUnit::from(&xunit);
        let reversed = CanonicalXUnit::from(&reversed);
        assert_eq!(
            encode_canonical_xunit(&canonical).unwrap(),
            encode_canonical_xunit(&reversed).unwrap()
        );
        assert_eq!(
            canonical_xunit_content_id(&canonical).unwrap(),
            canonical_xunit_content_id(&reversed).unwrap()
        );
    }

    #[test]
    fn canonical_golden_bytes_and_content_id_v1() {
        let canonical = CanonicalXUnit::from(&XUnit::new(vec![
            YPath::new("geo").with_attribute("country", "CZ"),
        ]));
        let bytes = encode_canonical_xunit(&canonical).unwrap();
        let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            hex,
            "43585501000000010000000367656f0000000100000007636f756e7472790600000002435a"
        );
        assert_eq!(
            canonical_xunit_content_id(&canonical).unwrap().to_hex(),
            "bed9c102177b3a386d76f139182452ae0f343c8760bf439d66aca37ad547fc4a"
        );
    }

    #[test]
    fn canonical_encoding_rejects_duplicate_dimensions_and_non_finite_floats() {
        let duplicate = CanonicalXUnit {
            ypaths: vec![
                CanonicalYPath {
                    dim: "x".into(),
                    attributes: Vec::new(),
                },
                CanonicalYPath {
                    dim: "x".into(),
                    attributes: Vec::new(),
                },
            ],
        };
        assert!(encode_canonical_xunit(&duplicate).is_err());

        let non_finite = CanonicalXUnit {
            ypaths: vec![CanonicalYPath {
                dim: "x".into(),
                attributes: vec![CanonicalAttribute {
                    name: "v".into(),
                    value: CanonicalValue::F64(f64::NAN),
                }],
            }],
        };
        assert!(encode_canonical_xunit(&non_finite).is_err());
    }
}
