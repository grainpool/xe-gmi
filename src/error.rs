//! Error type and the exit-code contract.
//!
//! Every failure path in the binary ends in exactly one of these variants, and every variant maps
//! to exactly one exit code. Clap usage errors exit 2 on their own.

use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum Error {
    /// Bad arguments detected after clap parsing (e.g. `set clocks` with neither --min nor --max). Exit 2.
    Usage(String),
    /// No xe-bound device usable (or the selected device does not exist). Exit 3.
    NoDevice(String),
    /// EACCES/EPERM on a write or a privileged read. `hint` is the exact command that would work. Exit 4.
    PermissionDenied { path: PathBuf, hint: String },
    /// The feature is not available on this kernel/card; message says what was looked for and where. Exit 5.
    Unavailable(String),
    /// A write was accepted but readback disagrees in an unexpected way, or the kernel rejected it. Exit 6.
    WriteFailed(String),
    /// Unexpected I/O failure not covered above (vanished file mid-run, etc). Exit 6.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Reserved for genuine internal bugs. Must never be reachable in a release. Exit 70.
    Internal(String),
}

impl Error {
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Usage(_) => 2,
            Error::NoDevice(_) => 3,
            Error::PermissionDenied { .. } => 4,
            Error::Unavailable(_) => 5,
            Error::WriteFailed(_) | Error::Io { .. } => 6,
            Error::Internal(_) => 70,
        }
    }

    /// Classify a raw I/O error from a sysfs write into the contract's variants.
    pub fn from_write(path: PathBuf, source: std::io::Error, sudo_hint: String) -> Error {
        use std::io::ErrorKind::*;
        match source.kind() {
            PermissionDenied => Error::PermissionDenied {
                path,
                hint: sudo_hint,
            },
            NotFound => Error::Unavailable(format!(
                "{} vanished while writing (driver reload, runtime suspend or unbind?)",
                path.display()
            )),
            // EINVAL from the driver (value out of [rpn, rp0], bad token) surfaces as InvalidInput.
            InvalidInput => Error::WriteFailed(format!(
                "kernel rejected the value written to {} (EINVAL)",
                path.display()
            )),
            // EOPNOTSUPP surfaces as Unsupported.
            Unsupported => Error::Unavailable(format!(
                "kernel reports the operation on {} is not supported (EOPNOTSUPP)",
                path.display()
            )),
            _ => Error::Io { path, source },
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(m) => write!(f, "{m}"),
            Error::NoDevice(m) => write!(f, "{m}"),
            Error::PermissionDenied { path, hint } => {
                write!(
                    f,
                    "permission denied writing {}\n  try: {}",
                    path.display(),
                    hint
                )
            }
            Error::Unavailable(m) => write!(f, "not available: {m}"),
            Error::WriteFailed(m) => write!(f, "write failed: {m}"),
            Error::Io { path, source } => write!(f, "{}: {}", path.display(), source),
            Error::Internal(m) => write!(f, "internal error: {m}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::Error;
    use std::io::{Error as IoError, ErrorKind};
    use std::path::PathBuf;

    fn from_kind(kind: ErrorKind) -> Error {
        Error::from_write(
            PathBuf::from("/sys/devices/x/hwmon/hwmon7/power1_cap"),
            IoError::from(kind),
            "sudo xe-gmi set power-limit 150".to_string(),
        )
    }

    #[test]
    fn from_write_maps_kinds() {
        assert_eq!(from_kind(ErrorKind::PermissionDenied).exit_code(), 4);
        assert_eq!(from_kind(ErrorKind::InvalidInput).exit_code(), 6);
        assert_eq!(from_kind(ErrorKind::Unsupported).exit_code(), 5);
        assert_eq!(from_kind(ErrorKind::NotFound).exit_code(), 5);
        assert_eq!(from_kind(ErrorKind::Other).exit_code(), 6);
        assert!(matches!(
            from_kind(ErrorKind::PermissionDenied),
            Error::PermissionDenied { .. }
        ));
        assert!(matches!(
            from_kind(ErrorKind::InvalidInput),
            Error::WriteFailed(_)
        ));
        assert!(matches!(
            from_kind(ErrorKind::Unsupported),
            Error::Unavailable(_)
        ));
        assert!(matches!(
            from_kind(ErrorKind::NotFound),
            Error::Unavailable(_)
        ));
        assert!(matches!(from_kind(ErrorKind::Other), Error::Io { .. }));
    }

    #[test]
    fn permission_message_carries_the_sudo_hint() {
        let s = from_kind(ErrorKind::PermissionDenied).to_string();
        assert!(s.starts_with("permission denied writing /sys/devices/x/hwmon/hwmon7/power1_cap"));
        assert!(s.contains("try: sudo xe-gmi set power-limit 150"));
    }

    #[test]
    fn exit_codes_are_the_contract() {
        assert_eq!(Error::Usage(String::new()).exit_code(), 2);
        assert_eq!(Error::NoDevice(String::new()).exit_code(), 3);
        assert_eq!(Error::Unavailable(String::new()).exit_code(), 5);
        assert_eq!(Error::WriteFailed(String::new()).exit_code(), 6);
        assert_eq!(Error::Internal(String::new()).exit_code(), 70);
    }
}
