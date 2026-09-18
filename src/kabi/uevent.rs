//! `NETLINK_KOBJECT_UEVENT` reader: the kernel uevent stream, plus the line format the replay
//! files use (`NUL` written as `|`, one event per line).

use std::path::Path;

use rustix::net::netlink::{SocketAddrNetlink, KOBJECT_UEVENT as NETLINK_KOBJECT_UEVENT_PROTOCOL};
use rustix::net::{recv, socket, AddressFamily, RecvFlags, SocketType};

use crate::error::Error;

pub const NETLINK_KOBJECT_UEVENT: u16 = 15;

#[derive(Debug, Clone, PartialEq)]
pub struct Uevent {
    pub action: String,
    pub devpath: String,
    pub env: Vec<(String, String)>,
}

/// One replay line: `action@devpath|KEY=VAL|KEY=VAL|…` (`|` stands where the kernel puts `NUL`).
pub fn parse_line(line: &str) -> Option<Uevent> {
    let mut it = line.split('|');
    parse_head(it.next()?).map(|(action, devpath)| Uevent {
        action,
        devpath,
        env: it.filter_map(parse_env).collect(),
    })
}

/// One live kernel message: `ACTION@DEVPATH\0KEY=VAL\0…`.
pub fn parse_message(bytes: &[u8]) -> Option<Uevent> {
    let mut it = bytes
        .split(|b| *b == 0)
        .map(|s| String::from_utf8_lossy(s).into_owned());
    let head = it.next()?;
    let (action, devpath) = head.split_once('@')?;
    Some(Uevent {
        action: action.to_string(),
        devpath: devpath.to_string(),
        env: it.filter_map(|s| parse_env(&s)).collect(),
    })
}

fn parse_head(head: &str) -> Option<(String, String)> {
    let (action, devpath) = head.split_once('@')?;
    Some((action.to_string(), devpath.to_string()))
}

fn parse_env(item: &str) -> Option<(String, String)> {
    let (k, v) = item.split_once('=')?;
    Some((k.to_string(), v.to_string()))
}

pub fn env<'a>(u: &'a Uevent, key: &str) -> Option<&'a str> {
    env_get(u, key)
}

fn env_get<'a>(u: &'a Uevent, key: &str) -> Option<&'a str> {
    u.env
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

pub fn subsystem(u: &Uevent) -> Option<&str> {
    env_get(u, "SUBSYSTEM")
}

pub fn pci_slot(u: &Uevent) -> Option<&str> {
    env_get(u, "PCI_SLOT_NAME")
}

/// `xxxx:yy:zz.n` address inside a DEVPATH (driver-bound events carry the function directory).
pub fn pci_in_devpath(u: &Uevent) -> Option<&str> {
    u.devpath.split('/').rfind(|s| is_pci_shape(s))
}

pub fn is_pci_shape(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 12
        && b[4] == b':'
        && b[7] == b':'
        && b[10] == b'.'
        && b[..4].iter().all(|c| c.is_ascii_hexdigit())
        && b[5..7].iter().all(|c| c.is_ascii_hexdigit())
        && b[8..10].iter().all(|c| c.is_ascii_hexdigit())
        && (b'0'..=b'7').contains(&b[11])
}

/// Blocking live stream (root only: binding the kernel multicast group needs the capability).
pub fn live_stream() -> Result<Box<dyn Iterator<Item = Uevent>>, Error> {
    let fd = socket(
        AddressFamily::NETLINK,
        SocketType::RAW,
        Some(NETLINK_KOBJECT_UEVENT_PROTOCOL),
    )
    .map_err(|e| live_error(e, Path::new("/run/xe-gmi/uevent")))?;
    rustix::net::bind(&fd, &SocketAddrNetlink::new(0, 1))
        .map_err(|e| live_error(e, Path::new("/run/xe-gmi/uevent")))?;
    let file = std::fs::File::from(fd);
    Ok(Box::new(Live {
        file,
        buf: vec![0u8; 8 * 1024],
    }))
}

fn live_error(e: rustix::io::Errno, path: &Path) -> Error {
    match e {
        rustix::io::Errno::PERM | rustix::io::Errno::ACCESS => Error::PermissionDenied {
            path: path.to_path_buf(),
            hint: "sudo xe-gmi events".into(),
        },
        other => Error::Io {
            path: path.to_path_buf(),
            source: other.into(),
        },
    }
}

struct Live {
    file: std::fs::File,
    buf: Vec<u8>,
}

impl Iterator for Live {
    type Item = Uevent;
    fn next(&mut self) -> Option<Uevent> {
        loop {
            let (_, n) = recv(&self.file, &mut self.buf[..], RecvFlags::empty()).ok()?;
            if let Some(u) = parse_message(&self.buf[..n]) {
                return Some(u);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uevent_parse_replay_line() {
        let u = parse_line(
            "change@/devices/…/0000:67:00.0/drm/card2|ACTION=change|SUBSYSTEM=drm|WEDGED=rebind,bus-reset",
        )
        .unwrap();
        assert_eq!(u.action, "change");
        assert!(u.devpath.ends_with("/drm/card2"));
        assert_eq!(env(&u, "WEDGED"), Some("rebind,bus-reset"));
        assert_eq!(subsystem(&u), Some("drm"));
    }

    #[test]
    fn uevent_parse_live_message() {
        let raw = b"bind@/devices/pci0000:64/0000:64:01.0/0000:65:00.0/0000:66:04.0/0000:67:00.0\0\
                    ACTION=bind\0SUBSYSTEM=pci\0DRIVER=xe\0PCI_SLOT_NAME=0000:67:00.0\0";
        let u = parse_message(raw).unwrap();
        assert_eq!(u.action, "bind");
        assert_eq!(pci_slot(&u), Some("0000:67:00.0"));
        assert_eq!(pci_in_devpath(&u), Some("0000:67:00.0"));
    }

    #[test]
    fn uevent_pci_shape() {
        assert!(is_pci_shape("0000:67:00.0"));
        assert!(!is_pci_shape("pci0000:64"));
        assert!(!is_pci_shape("0000:67:00.8"));
        assert!(!is_pci_shape("card2"));
    }
}
