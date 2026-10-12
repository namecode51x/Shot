//! Typed errors with human-readable messages. Win32/COM failures keep
//! their original code; normal OS failures never panic.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// Win32/COM failure surfaced by the `windows` crate (keeps HRESULT + system message).
    #[cfg(windows)]
    Win32(windows::core::Error),
    /// Filesystem failure with a short English context label.
    Io {
        context: &'static str,
        source: std::io::Error,
    },
    /// Invalid input (bad dimensions, bad settings value, bad hotkey...).
    Invalid(String),
    /// Anything else, already formatted for the user.
    Msg(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn msg<S: Into<String>>(s: S) -> Self {
        Error::Msg(s.into())
    }
    pub fn invalid<S: Into<String>>(s: S) -> Self {
        Error::Invalid(s.into())
    }
    pub fn io(context: &'static str, source: std::io::Error) -> Self {
        Error::Io { context, source }
    }
    /// Prefix a Win32 failure with the API that produced it.
    #[cfg_attr(not(windows), allow(unused_variables))]
    pub fn with_context(self, api: &'static str) -> Self {
        match self {
            #[cfg(windows)]
            Error::Win32(e) => Error::Msg(format!("{api}: {e}")),
            other => other,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(windows)]
            Error::Win32(e) => write!(f, "{e}"),
            Error::Io { context, source } => write!(f, "{context}: {source}"),
            Error::Invalid(m) => write!(f, "{m}"),
            Error::Msg(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(windows)]
impl From<windows::core::Error> for Error {
    fn from(e: windows::core::Error) -> Self {
        Error::Win32(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io {
            context: "I/O error",
            source: e,
        }
    }
}

pub fn context<'a, T>(r: Result<T>, what: &'a str) -> Result<T> {
    r.map_err(|e| Error::Msg(format!("{what}: {e}")))
}
