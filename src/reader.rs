//! One Reader = one modem (one PC/SC slot). Transport: TCP to a bridge
//! (socat) on the modem host.
//!
//! Protocol findings baked in (conwrt-bench #21/#22, RG502Q-EA):
//! - ATE1 echo-correlation: the echo of our command marks the boundary
//!   between stale/foreign traffic (URCs carry no echo) and our response.
//! - Pacing: back-to-back CSIM bursts trigger SIM-session thrash on some
//!   firmwares; a modest gap between transmits is cheap insurance. Presence
//!   probes must NOT be paced (pcscd polls presence frequently).

//! Transaction semantics (hardening Tasks 1.2/1.3): an AT command whose
//! line was fully written may already be executing on the modem/SIM —
//! such exchanges are NEVER re-sent. Only a write that provably failed
//! before the terminating CR (nothing executable left the host) may be
//! retried, exactly once.

use crate::at;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TRANSMIT_GAP_MS: u64 = 400;
const PRESENCE_CACHE_MS: u64 = 5000;
/// Hard bound on response accumulation: a CSIM answer is at most ~520
/// bytes of framing around ≤516 hex chars; everything beyond is a broken
/// or hostile bridge and the connection is dropped instead of buffering
/// without limit.
const READ_CAP: usize = 8192;

pub struct Reader {
    pub target: String,
    last_cmd: Mutex<Option<Instant>>,
    stream: Mutex<Option<TcpStream>>,
    presence_ts: Mutex<Option<Instant>>,
    present: Mutex<Option<bool>>,
}

/// Typed exchange outcome. The distinction between NotSent, Refused and
/// Ambiguous is what makes the retry policy safe:
///
/// - `NotSent` — the write failed before the terminating CR left the
///   host, so the modem holds at most an unexecutable partial line.
///   Retrying cannot duplicate anything.
/// - `Refused` — our echo came back (the modem provably heard us) but it
///   never produced a final result code (firmware-intercepted select,
///   silent refusal). Card-level outcome; retrying rarely helps.
/// - `Ambiguous` — the line was fully written but no echo was ever seen:
///   a stalled bridge or a death mid-relay is indistinguishable from
///   "the modem already ran the command". NEVER resend.
#[derive(Debug)]
pub enum AtOutcome {
    /// final result code OK received after a matched prefix
    Ok(String),
    /// modem answered final ERROR (definitive refusal)
    Error(String),
    /// write failed before the command could execute — safe to retry
    NotSent,
    /// modem heard us but never answered — card-level refusal (no retry)
    Refused,
    /// command fully written, outcome unknown — transport-level (no retry)
    Ambiguous,
}

fn append_log(msg: &str) {
    crate::log(msg);
}

/// APDU payloads are authentication/provisioning secrets: their hex is
/// logged only under IFD_MODEMIFD_LOG_APDU; the default records lengths.
fn log_apdu_content() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("IFD_MODEMIFD_LOG_APDU").is_ok())
}

/// Inter-command pacing floor. Default `TRANSMIT_GAP_MS`; override per
/// pcscd process via `IFD_MODMIFD_TX_GAP_MS=<ms>` (unparsable values
/// fall back to the default; `0` disables pacing entirely — see the
/// README's CSIM-thrash caveat before disabling). Cached once: the env
/// var must be set before the first paced transmit.
fn transmit_gap() -> Duration {
    static G: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    Duration::from_millis(*G.get_or_init(|| {
        std::env::var("IFD_MODMIFD_TX_GAP_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(TRANSMIT_GAP_MS)
    }))
}

impl Reader {
    pub fn new(target: &str) -> Reader {
        Reader {
            target: target.to_string(),
            last_cmd: Mutex::new(None),
            stream: Mutex::new(None),
            presence_ts: Mutex::new(None),
            present: Mutex::new(None),
        }
    }

    fn connect(&self) -> Option<TcpStream> {
        let (host, port) = self.target.rsplit_once(':')?;
        let port: u16 = port.parse().ok()?;
        let mut last_err = None;
        for addr in std::net::ToSocketAddrs::to_socket_addrs(&(host, port)).ok()? {
            match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
                Ok(s) => {
                    let _ = s.set_nodelay(true);
                    return Some(s);
                }
                Err(e) => last_err = Some(e),
            }
        }
        append_log(&format!("connect {} failed: {:?}", self.target, last_err));
        None
    }

    fn stream_slot(&self) -> std::sync::MutexGuard<'_, Option<TcpStream>> {
        self.stream.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// drain + optional pacing + write. Caller holds the stream guard.
    fn prime_and_send(&self, stream: &mut TcpStream, cmd: &str, pace: bool) -> Result<(), ()> {
        let _ = stream.set_nonblocking(true);
        let mut sink = [0u8; 2048];
        let deadline = Instant::now() + Duration::from_millis(120);
        while Instant::now() < deadline {
            match stream.read(&mut sink) {
                Ok(0) => break,
                Ok(_) => {}
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => return Err(()),
            }
        }
        let _ = stream.set_nonblocking(false);
        if pace {
            let mut lc = self.last_cmd.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(t) = *lc {
                let wait = transmit_gap().saturating_sub(t.elapsed());
                if !wait.is_zero() {
                    std::thread::sleep(wait);
                }
            }
            *lc = Some(Instant::now());
        }
        // The written line ends in CR; write_all failing means the CR
        // (the last byte) never left the host, so the modem cannot have
        // executed anything.
        stream
            .write_all(format!("{}\r", cmd).as_bytes())
            .map_err(|_| ())
    }

    /// Echo-correlated, prefix-matched exchange. `pace` inserts the
    /// inter-command gap (transmits: yes; presence probes: no).
    /// Post-write failures never classify as NotSent.
    fn at_full(&self, cmd: &str, prefix: &str, total: Duration, pace: bool) -> AtOutcome {
        let mut guard = self.stream_slot();
        if guard.is_none() {
            *guard = self.connect();
        }
        let stream = match guard.as_mut() {
            Some(s) => s,
            None => return AtOutcome::NotSent,
        };
        if self.prime_and_send(stream, cmd, pace).is_err() {
            *guard = None;
            return AtOutcome::NotSent;
        }
        let mut resp = Vec::with_capacity(2048);
        let start = Instant::now();
        let mut echo_off: Option<usize> = None;
        let mut saw_prefix = false;
        loop {
            if start.elapsed() >= total {
                // deadline hit: the echo discriminator separates a
                // proven-heard modem (Refused) from an unknown-delivery
                // one (Ambiguous); stale state is never reused
                *guard = None;
                return if echo_off.is_some() {
                    AtOutcome::Refused
                } else {
                    AtOutcome::Ambiguous
                };
            }
            let _ = stream.set_read_timeout(Some(Duration::from_millis(150)));
            let mut chunk = [0u8; 1024];
            match stream.read(&mut chunk) {
                Ok(0) => {
                    *guard = None;
                    return if echo_off.is_some() {
                        AtOutcome::Refused
                    } else {
                        AtOutcome::Ambiguous
                    };
                }
                Ok(n) => {
                    resp.extend_from_slice(&chunk[..n]);
                    if resp.len() > READ_CAP {
                        append_log("response cap exceeded — dropping connection");
                        *guard = None;
                        return if echo_off.is_some() {
                            AtOutcome::Refused
                        } else {
                            AtOutcome::Ambiguous
                        };
                    }
                    let s = String::from_utf8_lossy(&resp).into_owned();
                    if echo_off.is_none() {
                        if let Some(p) = s.find(cmd) {
                            echo_off = Some(p + cmd.len());
                        }
                    }
                    if let Some(off) = echo_off {
                        let tail = &s[off..];
                        if !saw_prefix && tail.contains(prefix) {
                            saw_prefix = true;
                        }
                        if saw_prefix {
                            if tail.contains("OK\r") {
                                return AtOutcome::Ok(s);
                            }
                            if tail.contains("ERROR") {
                                return AtOutcome::Error(s);
                            }
                        }
                    }
                }
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => {
                    *guard = None;
                    return if echo_off.is_some() {
                        AtOutcome::Refused
                    } else {
                        AtOutcome::Ambiguous
                    };
                }
            }
        }
    }

    /// Paced exchange with exactly one retry, and only when the first
    /// attempt provably never reached the modem (write failed). Any
    /// post-write outcome is final: a re-sent AT+CSIM could duplicate a
    /// state-changing APDU (GP secure channel, eSIM, PIN).
    pub fn at_expect(&self, cmd: &str, prefix: &str, total: Duration) -> AtOutcome {
        match self.at_full(cmd, prefix, total, true) {
            AtOutcome::NotSent => self.at_full(cmd, prefix, total, true),
            other => other,
        }
    }

    /// APDU exchange over AT+CSIM (basic + logical channels pass through).
    pub fn transmit(&self, apdu: &[u8]) -> Result<Vec<u8>, ()> {
        self.transmit_with_timeout(apdu, Duration::from_millis(8000))
    }

    /// Test-tunable variant of [`Reader::transmit`].
    pub fn transmit_with_timeout(&self, apdu: &[u8], total: Duration) -> Result<Vec<u8>, ()> {
        if apdu.len() > 512 || apdu.is_empty() {
            return Ok(vec![0x6F, 0x00]);
        }
        let hex = at::bin2hex(apdu);
        let cmd = format!("AT+CSIM={},\"{}\"", apdu.len() * 2, hex);
        if log_apdu_content() {
            append_log(&format!("XMIT {}", hex));
        } else {
            append_log(&format!("XMIT len={}", apdu.len()));
        }
        match self.at_expect(&cmd, "+CSIM:", total) {
            AtOutcome::Ok(resp) => {
                let parsed = at::parse_csim_in(&resp, &cmd);
                append_log(&format!(
                    "  RESP parsed={:?}",
                    parsed.as_ref().map(|v| v.len())
                ));
                // Verified exchange, unparsable content: card-level
                // failure, not a transport error (pySim-shell survives).
                Ok(parsed.unwrap_or_else(|| vec![0x6F, 0x00]))
            }
            // Card/reader-level refusals: the modem IS the reader; a
            // refusal or post-echo silence maps to SW 6F00 (v3.3
            // semantics — AID-probing tools must survive them).
            AtOutcome::Error(_) | AtOutcome::Refused => Ok(vec![0x6F, 0x00]),
            // Delivery unknown or provably failed: never a synthetic
            // card status — surface the transport failure to pcscd.
            AtOutcome::NotSent | AtOutcome::Ambiguous => Err(()),
        }
    }

    pub fn presence(&self) -> bool {
        {
            let ts = self.presence_ts.lock().unwrap_or_else(|e| e.into_inner());
            let p = self.present.lock().unwrap_or_else(|e| e.into_inner());
            if let (Some(t), Some(v)) = (ts.as_ref(), p.as_ref()) {
                if t.elapsed() < Duration::from_millis(PRESENCE_CACHE_MS) {
                    return *v;
                }
            }
        }
        let mut p = false;
        let mut got = false;
        // Unpaced read-only probe; a retry cannot duplicate state.
        if let AtOutcome::Ok(resp) =
            self.at_full("AT+CPIN?", "+CPIN:", Duration::from_millis(2500), false)
        {
            if resp.contains("+CPIN:") {
                p = true;
                got = true;
            }
        }
        if !got {
            *self.stream_slot() = None;
            if let AtOutcome::Ok(r2) =
                self.at_full("AT+CPIN?", "+CPIN:", Duration::from_millis(2500), false)
            {
                p = r2.contains("+CPIN:");
            }
        }
        *self.presence_ts.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        *self.present.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
        p
    }

    /// Session init (lpac-style capability probe). Echo ON for correlation.
    pub fn init_handshake(&self) -> bool {
        matches!(
            self.at_full("ATE1", "OK", Duration::from_millis(2000), false),
            AtOutcome::Ok(_)
        ) && matches!(
            self.at_full("AT+CSIM=?", "OK", Duration::from_millis(2500), false),
            AtOutcome::Ok(_)
        )
    }

    /// PC/SC clients expect a freshly reset card on SCardConnect. A modem
    /// cannot cold-reset its SIM on demand; the closest safe emulation is
    /// closing any leaked logical channels and re-selecting MF, so every
    /// session starts from the card's root state.
    pub fn normalize(&self) {
        for ch in [1u8, 2u8] {
            let _ = self.transmit(&[ch, 0x70, 0x80, ch, 0x00]);
        }
        let _ = self.transmit(&[0x00, 0xA4, 0x00, 0x0C, 0x02, 0x3F, 0x00]);
        append_log("normalize: channels closed, MF selected");
    }

    pub fn close(&self) {
        *self.stream_slot() = None;
    }
}
