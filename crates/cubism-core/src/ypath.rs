//! YPath and XUnit: the coordinates of the cube lattice.
//!
//! Semantics ported from the legacy Scala library (`YPath.scala`,
//! `XUnit.scala`). The string forms (`/geo/country=CZ/city=Prague` and the
//! comma-joined XUnit) are a parse/present boundary format only — engine
//! internals use the binary key encoding in [`crate::encoding`].

use crate::error::CubismError;
use std::fmt;
use std::str::FromStr;

/// String form of the global rollup XUnit.
pub const GLOBAL: &str = "/G";

/// One dimension's hierarchical coordinate: an ordered list of
/// `attribute=value` pairs under a dimension name. The attribute *sequence*
/// encodes drill-down depth within the dimension (country → city → district).
///
/// A `None` value means the attribute is present but unvalued (parsed from a
/// bare `attr` segment with no `=`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct YPath {
    pub dim: String,
    pub attributes: Vec<(String, Option<String>)>,
}

impl YPath {
    pub fn new(dim: impl Into<String>) -> Self {
        YPath { dim: dim.into(), attributes: Vec::new() }
    }

    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.push((name.into(), Some(value.into())));
        self
    }

    pub fn attribute_value(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_deref())
    }

    /// Drill-down depth: number of attribute levels.
    pub fn depth(&self) -> usize {
        self.attributes.len()
    }
}

impl fmt::Display for YPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}", self.dim)?;
        for (name, value) in &self.attributes {
            match value {
                Some(v) => write!(f, "/{}={}", name, scrub_value(v))?,
                None => write!(f, "/{}", name)?,
            }
        }
        Ok(())
    }
}

impl FromStr for YPath {
    type Err = CubismError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_ypath(s)
    }
}

/// One cell of the cube lattice: a conjunction of YPaths, at most one per
/// dimension. The empty XUnit is the global rollup `/G`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct XUnit {
    pub ypaths: Vec<YPath>,
}

impl XUnit {
    pub fn global() -> Self {
        XUnit { ypaths: Vec::new() }
    }

    pub fn new(ypaths: Vec<YPath>) -> Self {
        XUnit { ypaths }
    }

    pub fn is_global(&self) -> bool {
        self.ypaths.is_empty()
    }

    pub fn with_ypath(mut self, yp: YPath) -> Self {
        self.ypaths.push(yp);
        self
    }

    /// Sort YPaths by dimension so the same lattice cell always has one
    /// canonical representation (its serialized form is the group-by key).
    pub fn normalize(mut self) -> Self {
        self.ypaths.sort_by(|a, b| a.dim.cmp(&b.dim));
        self
    }

    pub fn num_dimensions(&self) -> usize {
        self.ypaths.len()
    }

    pub fn contains_dimension(&self, dim: &str) -> bool {
        self.ypaths.iter().any(|yp| yp.dim == dim)
    }

    pub fn for_dimension(&self, dim: &str) -> Option<&YPath> {
        self.ypaths.iter().find(|yp| yp.dim == dim)
    }

    pub fn without_dimension(&self, dim: &str) -> XUnit {
        XUnit { ypaths: self.ypaths.iter().filter(|yp| yp.dim != dim).cloned().collect() }
    }
}

impl fmt::Display for XUnit {
    /// Canonical (normalized) string form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_global() {
            return f.write_str(GLOBAL);
        }
        let normalized = self.clone().normalize();
        let mut first = true;
        for yp in &normalized.ypaths {
            if !first {
                f.write_str(",")?;
            }
            write!(f, "{yp}")?;
            first = false;
        }
        Ok(())
    }
}

impl FromStr for XUnit {
    type Err = CubismError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == GLOBAL {
            return Ok(XUnit::global());
        }
        if s.is_empty() {
            return Err(CubismError::XUnitParse { input: s.into(), reason: "empty string".into() });
        }
        let ypaths = s
            .split(',')
            .map(parse_ypath)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CubismError::XUnitParse { input: s.into(), reason: e.to_string() })?;
        Ok(XUnit { ypaths })
    }
}

/// Escape the structural characters `/`, `=`, `,` in attribute values.
///
/// Sentinel scheme kept byte-compatible with the legacy library so exported
/// legacy fixtures parse unchanged. Lossy if a raw value already contains a
/// sentinel sequence (`___`, `###`, `+++`) — spec validation should reject
/// such dimension values at ingest when exactness matters.
pub fn scrub_value(dirty: &str) -> String {
    dirty.replace('/', "___").replace('=', "###").replace(',', "+++")
}

/// Inverse of [`scrub_value`].
pub fn unscrub_value(scrubbed: &str) -> String {
    scrubbed.replace("___", "/").replace("###", "=").replace("+++", ",")
}

fn parse_ypath(s: &str) -> Result<YPath, CubismError> {
    let err = |reason: &str| CubismError::YPathParse { input: s.into(), reason: reason.into() };

    let rest = s.strip_prefix('/').ok_or_else(|| err("must start with '/'"))?;
    let mut segments = rest.split('/');
    let dim = segments.next().filter(|d| !d.is_empty()).ok_or_else(|| err("missing dimension name"))?;

    let attributes = segments
        .map(|seg| {
            if seg.is_empty() {
                return Err(err("empty attribute segment"));
            }
            match seg.split_once('=') {
                Some((name, value)) => Ok((name.to_string(), Some(unscrub_value(value)))),
                None => Ok((seg.to_string(), None)),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(YPath { dim: dim.to_string(), attributes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geo() -> YPath {
        YPath::new("geo").with_attribute("country", "CZ").with_attribute("city", "Prague")
    }

    #[test]
    fn ypath_display_and_parse_round_trip() {
        let yp = geo();
        assert_eq!(yp.to_string(), "/geo/country=CZ/city=Prague");
        assert_eq!("/geo/country=CZ/city=Prague".parse::<YPath>().unwrap(), yp);
    }

    #[test]
    fn ypath_scrubs_structural_chars() {
        let yp = YPath::new("url").with_attribute("path", "/a=b,c");
        let s = yp.to_string();
        assert_eq!(s, "/url/path=___a###b+++c");
        assert_eq!(s.parse::<YPath>().unwrap(), yp);
    }

    #[test]
    fn ypath_bare_attribute_parses_as_none() {
        let yp: YPath = "/geo/country".parse().unwrap();
        assert_eq!(yp.attributes, vec![("country".to_string(), None)]);
        assert_eq!(yp.to_string(), "/geo/country");
    }

    #[test]
    fn xunit_display_is_normalized() {
        let x = XUnit::new(vec![geo(), YPath::new("gender").with_attribute("gender", "F")]);
        let y = XUnit::new(vec![YPath::new("gender").with_attribute("gender", "F"), geo()]);
        assert_eq!(x.to_string(), y.to_string());
        assert_eq!(x.to_string(), "/gender/gender=F,/geo/country=CZ/city=Prague");
    }

    #[test]
    fn global_round_trip() {
        assert_eq!(XUnit::global().to_string(), GLOBAL);
        assert!(GLOBAL.parse::<XUnit>().unwrap().is_global());
    }

    #[test]
    fn xunit_parse_round_trip() {
        let s = "/gender/gender=F,/geo/country=CZ/city=Prague";
        let x: XUnit = s.parse().unwrap();
        assert_eq!(x.num_dimensions(), 2);
        assert_eq!(x.for_dimension("geo").unwrap().attribute_value("city"), Some("Prague"));
        assert_eq!(x.to_string(), s);
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!("geo/country=CZ".parse::<YPath>().is_err());
        assert!("".parse::<XUnit>().is_err());
        assert!("//x=1".parse::<YPath>().is_err());
    }

    #[test]
    fn without_dimension() {
        let x = XUnit::new(vec![geo(), YPath::new("gender").with_attribute("gender", "F")]);
        let stripped = x.without_dimension("geo");
        assert_eq!(stripped.num_dimensions(), 1);
        assert!(!stripped.contains_dimension("geo"));
    }
}
