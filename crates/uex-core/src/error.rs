use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("invalid archive magic: found {found:#010x}")]
    Magic { found: u32 },

    #[error("unsupported pak version {version}")]
    UnsupportedPakVersion { version: u32 },

    #[error("malformed archive: {0}")]
    Malformed(String),

    #[error("archive is encrypted but no AES key was supplied")]
    EncryptedNoKey,

    #[error("AES key for GUID {guid} not supplied")]
    MissingKeyForGuid { guid: String },

    #[error("decompression failed for {method}")]
    DecompressionFailed { method: &'static str },

    #[error("Oodle: {0}")]
    Oodle(String),

    #[error("asset parse failed: {0}")]
    AssetParse(String),

    #[error("unsupported feature: {0}")]
    Unsupported(String),

    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),

    #[error("entry not found: {0}")]
    NotFound(String),

    #[error("{0}")]
    Other(String),

    #[error("serialization: {0}")]
    Serialization(String),

    #[error("image: {0}")]
    Image(String),
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Serialization(e.to_string())
    }
}

impl From<image::ImageError> for Error {
    fn from(e: image::ImageError) -> Self {
        Error::Image(e.to_string())
    }
}

impl Error {
    pub fn malformed(msg: impl Into<String>) -> Self {
        Error::Malformed(msg.into())
    }
    pub fn unsupported(msg: impl Into<String>) -> Self {
        Error::Unsupported(msg.into())
    }
    pub fn asset(msg: impl Into<String>) -> Self {
        Error::AssetParse(msg.into())
    }
    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
}
