use std::fmt;

/// Everything that can go wrong while reading, cleaning or writing a document.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The bytes are not a well-formed PSD/PSB (or a part of one).
    #[error("malformed {what}: {detail}")]
    Format { what: &'static str, detail: String },
    /// A structure we can read but deliberately refuse to rewrite.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The caller asked for something that does not exist or is ambiguous.
    #[error("{0}")]
    Invalid(String),
    /// An allocation sized by the input failed or a size overflowed.
    #[error("too large: {0}")]
    TooLarge(&'static str),
    /// The run was stopped by the user.
    #[error("cancelled")]
    Cancelled,
    /// File-system error.
    #[error("{context}: {source}")]
    Io { context: String, source: std::io::Error },
}

impl Error {
    pub(crate) fn format(what: &'static str, detail: impl fmt::Display) -> Self {
        Error::Format { what, detail: detail.to_string() }
    }

    pub(crate) fn io(context: impl fmt::Display, source: std::io::Error) -> Self {
        Error::Io { context: context.to_string(), source }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A `Vec` with fallible, input-sized reservation.
pub(crate) fn vec_with_capacity(n: usize) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| Error::TooLarge("output buffer"))?;
    Ok(v)
}
