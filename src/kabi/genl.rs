//! Generic-netlink framing and the drm-ras client. Safe code end to end: the sockets come from
//! `rustix::net` and every frame builder / reply parser is plain byte manipulation (the
//! round-trip is unit-tested against the bytes documented in the kernel's `drm_ras.c`).

use std::path::PathBuf;

use rustix::io::Errno;
use rustix::net::netlink::{SocketAddrNetlink, GENERIC as NETLINK_GENERIC_PROTOCOL};
use rustix::net::{recv, send, socket, AddressFamily, RecvFlags, SendFlags, SocketType};

use crate::error::Error;

pub const NETLINK_GENERIC: u16 = 16;
pub const NLCTRL_FAMILY_ID: u16 = 1;
pub const NLMSG_ERROR: u16 = 2;
pub const NLMSG_DONE: u16 = 3;
pub const NLM_F_REQUEST: u16 = 0x0001;
pub const NLM_F_ACK: u16 = 0x0004;
pub const NLM_F_DUMP: u16 = 0x0300;

pub const DRMS_RAS_FAMILY: &str = "drm-ras";
pub const CTRL_CMD_GETFAMILY: u8 = 3;
pub const CTRL_ATTR_FAMILY_ID: u16 = 1;
pub const CTRL_ATTR_FAMILY_NAME: u16 = 2;

pub const RAS_CMD_LIST_NODES: u8 = 1;
pub const RAS_CMD_GET_ERROR_COUNTER: u8 = 2;
pub const RAS_CMD_CLEAR_ERROR_COUNTER: u8 = 3;
pub const RAS_ATTR_NODE_ID: u16 = 1;
pub const RAS_ATTR_DEVICE_NAME: u16 = 2;
pub const RAS_ATTR_NODE_NAME: u16 = 3;
pub const RAS_ATTR_NODE_TYPE: u16 = 4;
pub const RAS_ATTR_ERROR_ID: u16 = 2;
pub const RAS_ATTR_ERROR_NAME: u16 = 3;
pub const RAS_ATTR_ERROR_VALUE: u16 = 4;

/// Parsed attributes of one reply message: `(type, unpadded payload)` pairs.
pub type Attrs = Vec<(u16, Vec<u8>)>;

/// RAS nodes with counters, filtered to one device.
pub type RasSnapshot = Vec<(RasNode, Vec<RasCounter>)>;

#[derive(Debug, Clone, PartialEq)]
pub struct RasNode {
    pub id: u32,
    pub device: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RasCounter {
    pub id: u32,
    pub name: String,
    pub value: u32,
}

fn nla(kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&((payload.len() + 4) as u16).to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(payload);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out
}

fn nla_u32(kind: u16, value: u32) -> Vec<u8> {
    nla(kind, &value.to_le_bytes())
}

fn nla_str(kind: u16, value: &str) -> Vec<u8> {
    let mut p = value.as_bytes().to_vec();
    p.push(0);
    nla(kind, &p)
}

/// One `nlmsghdr` + `genlmsghdr` + attributes request frame.
pub fn genl_request(family: u16, flags: u16, seq: u32, cmd: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0; 4]); // length, patched below
    out.extend_from_slice(&family.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.push(cmd);
    out.push(1); // genl protocol version
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(body);
    let len = out.len() as u32;
    out[..4].copy_from_slice(&len.to_le_bytes());
    out
}

pub fn get_family_request(seq: u32) -> Vec<u8> {
    genl_request(
        NLCTRL_FAMILY_ID,
        NLM_F_REQUEST | NLM_F_ACK,
        seq,
        CTRL_CMD_GETFAMILY,
        &nla_str(CTRL_ATTR_FAMILY_NAME, DRMS_RAS_FAMILY),
    )
}

pub fn list_nodes_request(family: u16, seq: u32) -> Vec<u8> {
    genl_request(
        family,
        NLM_F_REQUEST | NLM_F_DUMP,
        seq,
        RAS_CMD_LIST_NODES,
        &[],
    )
}

pub fn get_counters_request(family: u16, seq: u32, node: u32) -> Vec<u8> {
    genl_request(
        family,
        NLM_F_REQUEST | NLM_F_DUMP,
        seq,
        RAS_CMD_GET_ERROR_COUNTER,
        &nla_u32(RAS_ATTR_NODE_ID, node),
    )
}

pub fn clear_counter_request(family: u16, seq: u32, node: u32, error: u32) -> Vec<u8> {
    genl_request(
        family,
        NLM_F_REQUEST | NLM_F_ACK,
        seq,
        RAS_CMD_CLEAR_ERROR_COUNTER,
        &[
            nla_u32(RAS_ATTR_NODE_ID, node),
            nla_u32(RAS_ATTR_ERROR_ID, error),
        ]
        .concat(),
    )
}

/// One netlink message inside a receive buffer.
#[derive(Debug)]
pub struct NlMsg<'a> {
    pub kind: u16,
    pub seq: u32,
    pub payload: &'a [u8], // after the 16-byte nlmsghdr (genl messages start with a 4-byte genlmsghdr)
}

pub fn nl_messages(buf: &[u8]) -> Vec<NlMsg<'_>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 16 <= buf.len() {
        let len = u32::from_le_bytes(buf[at..at + 4].try_into().unwrap()) as usize;
        if len < 16 || at + len > buf.len() {
            break;
        }
        out.push(NlMsg {
            kind: u16::from_le_bytes(buf[at + 4..at + 6].try_into().unwrap()),
            seq: u32::from_le_bytes(buf[at + 8..at + 12].try_into().unwrap()),
            payload: &buf[at + 16..at + len],
        });
        at += len;
    }
    out
}

/// Attribute iterator over genl payload (skips the 4-byte genlmsghdr); returns each
/// `(type, payload)` pair, payload unpadded.
pub fn genl_attrs(payload: &[u8]) -> Attrs {
    attrs_after(payload, 4)
}

pub fn attrs_after(payload: &[u8], skip: usize) -> Attrs {
    let mut out = Vec::new();
    let mut at = skip;
    while at + 4 <= payload.len() {
        let len = u16::from_le_bytes(payload[at..at + 2].try_into().unwrap()) as usize;
        if len < 4 || at + len > payload.len() {
            break;
        }
        let kind = u16::from_le_bytes(payload[at + 2..at + 4].try_into().unwrap());
        out.push((kind, payload[at + 4..at + len].to_vec()));
        let mut next = at + len;
        while next % 4 != 0 && next < payload.len() {
            next += 1;
        }
        at = next;
    }
    out
}

fn attr_u32(attrs: &Attrs, kind: u16) -> Option<u32> {
    attrs
        .iter()
        .find(|(k, _)| *k == kind)
        .and_then(|(_, v)| <[u8; 4]>::try_from(v.as_slice()).ok())
        .map(u32::from_le_bytes)
}

fn attr_u16(attrs: &Attrs, kind: u16) -> Option<u16> {
    attrs
        .iter()
        .find(|(k, _)| *k == kind)
        .and_then(|(_, v)| <[u8; 2]>::try_from(v.as_slice()).ok())
        .map(u16::from_le_bytes)
}

fn attr_str(attrs: &Attrs, kind: u16) -> Option<String> {
    attrs
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, v)| String::from_utf8_lossy(trim_nul(v)).into_owned())
}

fn trim_nul(v: &[u8]) -> &[u8] {
    match v.iter().position(|b| *b == 0) {
        Some(n) => &v[..n],
        None => v,
    }
}

fn errno_to_error(e: Errno, hint_cmd: &str) -> Error {
    match e {
        Errno::NOENT | Errno::NOTSUP => Error::Unavailable(
            "drm-ras netlink family not present (kernel 7.2+ with xe RAS support)".into(),
        ),
        Errno::PERM | Errno::ACCESS => Error::PermissionDenied {
            path: PathBuf::from("/run/xe-gmi/drm-ras"),
            hint: format!("sudo xe-gmi {hint_cmd}"),
        },
        other => Error::Io {
            path: PathBuf::from("/run/xe-gmi/drm-ras"),
            source: other.into(),
        },
    }
}

fn open_genl() -> Result<std::fs::File, Error> {
    let fd = socket(
        AddressFamily::NETLINK,
        SocketType::RAW,
        Some(NETLINK_GENERIC_PROTOCOL),
    )
    .map_err(|e| errno_to_error(e, "ras"))?;
    rustix::net::bind(&fd, &SocketAddrNetlink::new(0, 0)).map_err(|e| errno_to_error(e, "ras"))?;
    // the protocol should terminate by itself (see take_replies); this is the safety net: a
    // misbehaving family must turn into a clean error, not a wedged control tool
    rustix::net::sockopt::set_socket_timeout(
        &fd,
        rustix::net::sockopt::Timeout::Recv,
        Some(std::time::Duration::from_secs(10)),
    )
    .map_err(|e| errno_to_error(e, "ras"))?;
    Ok(std::fs::File::from(fd))
}

/// What one receive buffer told us about the round trip.
#[derive(Debug, PartialEq)]
pub enum Round {
    More,
    Done,
}

/// Collect replies from one receive buffer. Dumps terminate on `NLMSG_DONE`; an
/// `NLMSG_ERROR` terminates the round trip in every case — with the carried errno when it is
/// non-zero, or as the ACK of an `NLM_F_ACK` request when it is zero. A zero code is TERMINAL:
/// a kernel with the drm-ras family absent answers get-family with nothing but a bare
/// `NLMSG_ERROR(0)` (verified on 7.1.13), and a loop that waits for `NLMSG_DONE` after that
/// blocks forever. Err(I32_MIN) marks a zero-code ACK (Done, no errno).
pub fn take_replies(buf: &[u8], replies: &mut Vec<Attrs>) -> Result<Round, i32> {
    for msg in nl_messages(buf) {
        match msg.kind {
            NLMSG_DONE => return Ok(Round::Done),
            NLMSG_ERROR => {
                let code = i32::from_le_bytes(msg.payload[..4].try_into().unwrap_or([0; 4]));
                if code != 0 {
                    return Err(code);
                }
                return Ok(Round::Done);
            }
            _ => {
                if !msg.payload.is_empty() {
                    replies.push(genl_attrs(msg.payload));
                }
            }
        }
    }
    Ok(Round::More)
}

/// Send one request, collect replies until the round trip terminates; `(kind, attrs)` per reply.
fn round_trip(fd: &std::fs::File, req: &[u8], hint_cmd: &str) -> Result<Vec<Attrs>, Error> {
    send(fd, req, SendFlags::empty()).map_err(|e| errno_to_error(e, hint_cmd))?;
    let mut replies = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match recv(fd, &mut buf[..], RecvFlags::empty()) {
            Ok((_, len)) => len,
            Err(Errno::AGAIN | Errno::TIMEDOUT) => {
                return Err(Error::Unavailable(format!(
                    "drm-ras netlink round trip timed out ({hint_cmd})"
                )))
            }
            Err(e) => return Err(errno_to_error(e, hint_cmd)),
        };
        match take_replies(&buf[..n], &mut replies) {
            Ok(Round::Done) => return Ok(replies),
            Ok(Round::More) => {}
            Err(code) => {
                let e = Errno::from_raw_os_error(code);
                return Err(errno_to_error(e, hint_cmd));
            }
        }
    }
}

fn resolve_family(fd: &std::fs::File) -> Result<u16, Error> {
    let replies = round_trip(fd, &get_family_request(1), "ras")?;
    for attrs in replies {
        if let Some(id) = attr_u16(&attrs, CTRL_ATTR_FAMILY_ID) {
            return Ok(id);
        }
    }
    Err(Error::Unavailable(
        "drm-ras netlink family not present (kernel 7.2+ with xe RAS support)".into(),
    ))
}

pub fn ras_nodes() -> Result<Vec<RasNode>, Error> {
    let fd = open_genl()?;
    let family = resolve_family(&fd)?;
    let replies = round_trip(&fd, &list_nodes_request(family, 2), "ras")?;
    let mut out = Vec::new();
    for attrs in replies {
        if let (Some(id), Some(device), Some(name)) = (
            attr_u32(&attrs, RAS_ATTR_NODE_ID),
            attr_str(&attrs, RAS_ATTR_DEVICE_NAME),
            attr_str(&attrs, RAS_ATTR_NODE_NAME),
        ) {
            out.push(RasNode { id, device, name });
        }
    }
    Ok(out)
}

pub fn ras_counters(node: u32) -> Result<Vec<RasCounter>, Error> {
    let fd = open_genl()?;
    let family = resolve_family(&fd)?;
    let replies = round_trip(&fd, &get_counters_request(family, 2, node), "ras")?;
    let mut out = Vec::new();
    for attrs in replies {
        if let (Some(id), Some(name), Some(value)) = (
            attr_u32(&attrs, RAS_ATTR_ERROR_ID),
            attr_str(&attrs, RAS_ATTR_ERROR_NAME),
            attr_u32(&attrs, RAS_ATTR_ERROR_VALUE),
        ) {
            out.push(RasCounter { id, name, value });
        }
    }
    Ok(out)
}

pub fn ras_clear(node: u32, error: u32) -> Result<(), Error> {
    let fd = open_genl()?;
    let family = resolve_family(&fd)?;
    round_trip(
        &fd,
        &clear_counter_request(family, 2, node, error),
        "ras --clear",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(kind: u16, seq: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&((payload.len() + 16) as u32).to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&seq.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }

    #[test]
    fn genl_frame_bytes_are_stable() {
        // nlmsghdr(16) + genlmsghdr(4) + NLA {len=12,type=2,"drm-ras\0"}
        let req = get_family_request(7);
        let mut want: Vec<u8> = vec![32, 0, 0, 0, 1, 0, 5, 0, 7, 0, 0, 0, 0, 0, 0, 0, 3, 1, 0, 0];
        want.extend_from_slice(&[12, 0, 2, 0]);
        want.extend_from_slice(b"drm-ras\0");
        assert_eq!(req, want);

        let dump = list_nodes_request(42, 9);
        assert_eq!(
            dump,
            vec![20, 0, 0, 0, 42, 0, 1, 3, 9, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0]
        );

        let counters = get_counters_request(42, 2, 7);
        assert_eq!(
            counters,
            vec![
                28, 0, 0, 0, 42, 0, 1, 3, 2, 0, 0, 0, 0, 0, 0, 0, 2, 1, 0, 0, 8, 0, 1, 0, 7, 0, 0,
                0
            ]
        );

        let clear = clear_counter_request(42, 3, 1, 2);
        assert_eq!(clear.len(), 36);
        assert_eq!(&clear[..4], &36u32.to_le_bytes()[..]);
        assert_eq!(
            &clear[16..],
            &[3, 1, 0, 0, 8, 0, 1, 0, 1, 0, 0, 0, 8, 0, 2, 0, 2, 0, 0, 0][..]
        );
    }

    #[test]
    fn genl_reply_parse_reads_nodes_and_counters() {
        let mut node_msg = vec![0u8; 4]; // genlmsghdr
        node_msg.extend_from_slice(&nla_u32(RAS_ATTR_NODE_ID, 1));
        node_msg.extend_from_slice(&nla_str(RAS_ATTR_DEVICE_NAME, "0000:17:00.0"));
        node_msg.extend_from_slice(&nla_str(RAS_ATTR_NODE_NAME, "correctable-errors"));
        let mut node_msg2 = vec![0u8; 4];
        node_msg2.extend_from_slice(&nla_u32(RAS_ATTR_NODE_ID, 2));
        node_msg2.extend_from_slice(&nla_str(RAS_ATTR_DEVICE_NAME, "0000:17:00.0"));
        node_msg2.extend_from_slice(&nla_str(RAS_ATTR_NODE_NAME, "uncorrectable-errors"));
        let done = msg(NLMSG_DONE, 2, &[0; 4]);
        let mut buf = msg(1, 2, &node_msg);
        buf.extend_from_slice(&msg(1, 2, &node_msg2));
        buf.extend_from_slice(&done);

        let msgs = nl_messages(&buf);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2].kind, NLMSG_DONE);
        let attrs = genl_attrs(msgs[0].payload);
        assert_eq!(attr_u32(&attrs, RAS_ATTR_NODE_ID), Some(1));
        assert_eq!(
            attr_str(&attrs, RAS_ATTR_NODE_NAME).as_deref(),
            Some("correctable-errors")
        );

        let mut ctr = vec![0u8; 4];
        ctr.extend_from_slice(&nla_u32(RAS_ATTR_ERROR_ID, 2));
        ctr.extend_from_slice(&nla_str(RAS_ATTR_ERROR_NAME, "soc-internal"));
        ctr.extend_from_slice(&nla_u32(RAS_ATTR_ERROR_VALUE, 3));
        let buf2 = msg(1, 2, &ctr);
        let attrs = genl_attrs(nl_messages(&buf2)[0].payload);
        assert_eq!(attr_u32(&attrs, RAS_ATTR_ERROR_VALUE), Some(3));
        assert_eq!(
            attr_str(&attrs, RAS_ATTR_ERROR_NAME).as_deref(),
            Some("soc-internal")
        );
    }

    fn error_msg(seq: u32, code: i32) -> Vec<u8> {
        msg(NLMSG_ERROR, seq, &code.to_le_bytes())
    }

    #[test]
    fn genl_bare_error_zero_terminates() {
        // the 7.1.13 case: family absent -> kernel answers with nothing but NLMSG_ERROR(0).
        // the old round trip waited for NLMSG_DONE here and wedged the whole tool.
        let mut replies: Vec<Attrs> = Vec::new();
        assert_eq!(
            take_replies(&error_msg(7, 0), &mut replies),
            Ok(Round::Done)
        );
        assert!(replies.is_empty());
    }

    #[test]
    fn genl_record_then_ack_terminates() {
        let mut record = vec![0u8; 4]; // genlmsghdr
        record.extend_from_slice(&nla_u16(CTRL_ATTR_FAMILY_ID, 42));
        record.extend_from_slice(&nla_str(CTRL_ATTR_FAMILY_NAME, DRMS_RAS_FAMILY));
        let mut buf = msg(NLMSG_DONE.min(CTRL_ATTR_FAMILY_ID), 7, &record); // kind 1: a reply
        buf.extend_from_slice(&error_msg(7, 0)); // the ACK
        let mut replies: Vec<Attrs> = Vec::new();
        assert_eq!(take_replies(&buf, &mut replies), Ok(Round::Done));
        assert_eq!(replies.len(), 1);
        assert_eq!(attr_u16(&replies[0], CTRL_ATTR_FAMILY_ID), Some(42));
    }

    #[test]
    fn genl_dump_terminates_on_done_across_buffers() {
        let mut rec = vec![0u8; 4];
        rec.extend_from_slice(&nla_u32(RAS_ATTR_NODE_ID, 1));
        let first = msg(1, 2, &rec); // buffer 1: one record, no terminator yet
        let mut replies: Vec<Attrs> = Vec::new();
        assert_eq!(take_replies(&first, &mut replies), Ok(Round::More));
        assert_eq!(replies.len(), 1);
        let done = msg(NLMSG_DONE, 2, &[0; 4]);
        assert_eq!(take_replies(&done, &mut replies), Ok(Round::Done));
        assert_eq!(replies.len(), 1);
    }

    #[test]
    fn genl_error_code_propagates() {
        let mut replies: Vec<Attrs> = Vec::new();
        assert_eq!(take_replies(&error_msg(7, -1), &mut replies), Err(-1)); // EPERM
    }

    fn nla_u16(kind: u16, value: u16) -> Vec<u8> {
        nla(kind, &value.to_le_bytes())
    }

    #[test]
    fn genl_error_mapping() {
        match errno_to_error(Errno::NOENT, "ras") {
            Error::Unavailable(m) => assert!(m.contains("drm-ras") && m.contains("7.2")),
            other => panic!("{other:?}"),
        }
        match errno_to_error(Errno::PERM, "ras --clear") {
            Error::PermissionDenied { hint, .. } => assert_eq!(hint, "sudo xe-gmi ras --clear"),
            other => panic!("{other:?}"),
        }
    }
}
