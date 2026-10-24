//! vpcd wire protocol (vsmartcard virtual reader): every frame is a
//! 2-byte big-endian length + payload. One-byte payloads are controls —
//! OFF (0x00) is fire-and-forget and must NEVER be answered, ATR (0x02)
//! and RESET (0x04) are each answered with exactly one ATR frame. Any
//! longer payload is a C-APDU answered with one R-APDU frame. Reference
//! implementation: org.amperstrand.VpcdBridge (ccid-firmware-rs), plus
//! tests/hardware/VPCD-ARCHITECTURE.md.

pub const CTRL_OFF: u8 = 0x00;
pub const CTRL_ATR: u8 = 0x02;
pub const CTRL_RESET: u8 = 0x04;

#[derive(Debug, PartialEq, Eq)]
pub enum Frame {
    Control(u8),
    Apdu(Vec<u8>),
}

pub fn encode(payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut out = Vec::with_capacity(2 + payload.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Total frame size (header + payload) the buffer must reach before
/// `decode` can succeed; None while the header is incomplete.
pub fn expected(buf: &[u8]) -> Option<usize> {
    if buf.len() < 2 {
        return None;
    }
    let len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
    Some(2 + len)
}

/// Decode one frame from the front of `buf`, returning it and how many
/// bytes it consumed. Zero-length payloads are protocol noise and decode
/// as `None` with the caller expected to skip one byte (only reachable
/// from a hostile/desynced peer).
pub fn decode(buf: &[u8]) -> Option<(Frame, usize)> {
    let total = expected(buf)?;
    if buf.len() < total {
        return None;
    }
    let payload = &buf[2..total];
    if payload.len() == 1 {
        Some((Frame::Control(payload[0]), total))
    } else if payload.len() >= 2 {
        Some((Frame::Apdu(payload.to_vec()), total))
    } else {
        None
    }
}
