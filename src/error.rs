use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("XML parsing error: {0}")]
    Xml(#[from] quick_xml::DeError),
    #[error("missing required element: {0}")]
    MissingElement(&'static str),
    #[error("invalid attribute value for `{attr}`: {value}")]
    InvalidAttribute { attr: &'static str, value: String },
    #[error("unsupported schema version: {0}")]
    UnsupportedVersion(String),
}

pub type Result<T> = std::result::Result<T, Error>;
