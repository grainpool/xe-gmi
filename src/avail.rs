//! `Avail<T>` and the reasons behind every `N/A`.

use std::fmt;
use std::io::ErrorKind;
use std::path::PathBuf;

/// Why a datum is not available. Display renders the `<kind>: <detail>` text used by `-v`.
#[derive(Debug, Clone, PartialEq)]
pub enum Reason {
    Missing(PathBuf),
    Unreadable(PathBuf, ErrorKind),
    Unparseable(PathBuf, String),
    NotSupported(&'static str),
    /// A reason carrying its full sentence (kernel-interface access, for instance).
    Detail(String),
    NeedsRoot(PathBuf),
    FirstSample,
    KernelTooOld {
        feature: &'static str,
        first: &'static str,
    },
}

/// Every readable datum is either a value or a reason.
#[derive(Debug, Clone, PartialEq)]
pub enum Avail<T> {
    Value(T),
    NotAvailable(Reason),
}

impl<T> Avail<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Avail::Value(v) => Some(v),
            Avail::NotAvailable(_) => None,
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Avail<U> {
        match self {
            Avail::Value(v) => Avail::Value(f(v)),
            Avail::NotAvailable(r) => Avail::NotAvailable(r),
        }
    }

    pub fn or_reason(&self) -> Result<&T, &Reason> {
        match self {
            Avail::Value(v) => Ok(v),
            Avail::NotAvailable(r) => Err(r),
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::Missing(p) => write!(f, "missing: {}", p.display()),
            Reason::Unreadable(p, k) => write!(f, "unreadable: {} ({k:?})", p.display()),
            Reason::Unparseable(p, raw) => write!(f, "unparseable: {} = {raw}", p.display()),
            Reason::NotSupported(t) => write!(f, "not supported: {t}"),
            Reason::Detail(t) => write!(f, "{t}"),
            Reason::NeedsRoot(p) => write!(f, "needs root: {}", p.display()),
            Reason::FirstSample => write!(f, "first sample"),
            Reason::KernelTooOld { feature, first } => {
                write!(f, "kernel too old: {feature} needs {first}+")
            }
        }
    }
}
