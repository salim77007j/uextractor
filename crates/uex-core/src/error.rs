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

    /// A read of a specific entry failed. `hint` carries an actionable diagnosis
    /// (e.g. "this entry is AES-encrypted and the key is missing or wrong").
    #[error("failed to read '{entry}': {cause}{hint}")]
    EntryRead {
        entry: String,
        cause: String,
        hint: String,
    },

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

    /// Wrap a low-level read error with entry context and an actionable hint.
    ///
    /// The most common cause of a decompression failure on a UE archive is a
    /// missing or wrong AES key: encrypted blocks decrypt to high-entropy garbage
    /// and every decompressor rejects them. Detect that case and say so plainly.
    pub fn entry_read(entry: &str, cause: &Error, encrypted: bool, had_key: bool) -> Self {
        let hint = if encrypted {
            if had_key {
                " — this entry is AES-encrypted: the supplied key is most likely WRONG. \
                 Verify the 64-hex-char key for this game (🔑 AES keys) and try again."
                    .to_string()
            } else {
                " — this entry is AES-encrypted but NO key is loaded. \
                 Open 🔑 AES keys, add the game's 64-hex-char encryption key, then reload the archive."
                    .to_string()
            }
        } else if matches!(
            cause,
            Error::DecompressionFailed { .. } | Error::Oodle(_)
        ) {
            " — the data is not valid compressed data. Either the archive is damaged/incomplete, \
             or the game encrypts entries without setting the encrypted flag (a custom AES key may still be required)."
                .to_string()
        } else {
            String::new()
        };
        Error::EntryRead {
            entry: entry.to_string(),
            cause: cause.to_string(),
            hint,
        }
    }
}
