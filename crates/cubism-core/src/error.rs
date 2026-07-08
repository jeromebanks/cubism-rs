use thiserror::Error;

#[derive(Debug, Error)]
pub enum CubismError {
    #[error("cannot parse YPath '{input}': {reason}")]
    YPathParse { input: String, reason: String },

    #[error("cannot parse XUnit '{input}': {reason}")]
    XUnitParse { input: String, reason: String },

    #[error("cannot decode XUnit key: {0}")]
    Decode(String),

    #[error("invalid cube spec:\n{}", .0.join("\n"))]
    Validation(Vec<String>),

    #[error("cannot parse cube spec: {0}")]
    SpecParse(String),
}
