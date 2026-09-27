//! One Reader = one modem (one PC/SC slot). Transport: TCP to a bridge
//! (socat) on the modem host.
//!
//! Protocol findings baked in (conwrt-bench #21/#22, RG502Q-EA):
//! - ATE1 echo-correlation: the echo of our command marks the boundary
//!   between stale/foreign traffic (URCs carry no echo) and our response.
//! - Pacing: back-to-back CSIM bursts trigger SIM-session thrash on some
//!   firmwares; a modest gap between transmits is cheap insurance. Presence
//!   probes must NOT be paced (pcscd polls presence frequently).

use crate::at;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TRANSMIT_GAP_MS: u64 = 400;
const PRESENCE_CACHE_MS: u64 = 5000;

pub struct Reader {
    pub target: String,
    last_cmd: Mutex<Option<Instant>>,
    stream: Mutex<Option<TcpStream>>,
    presence_ts: Mutex<Option<Instant>>,
    present: Mutex<Option<bool>>,
}

pub enum AtResult {
    Ok(String),
    Err(String),
    Transport,
}

fn append_log(msg: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("/tmp/ifd-modemifd-reader.log") {
        let _ = writeln!(f, "{}", msg);
    }
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
    fn prime_and_send(
        &self,
        stream: &mut TcpStream,
        cmd: &str,
        pace: bool,
    ) -> Result<(), ()> {
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
                let wait = Duration::from_millis(TRANSMIT_GAP_MS).saturating_sub(t.elapsed());
                if !wait.is_zero() {
                    std::thread::sleep(wait);
                }
            }
            *lc = Some(Instant::now());
        }
        stream
            .write_all(format!("{}\r", cmd).as_bytes())
            .map_err(|_| ())
    }

    /// Echo-correlated, prefix-matched exchange. `pace` inserts the
    /// inter-command gap (transmits: yes; presence probes: no).
    fn at_full(&self, cmd: &str, prefix: &str, total: Duration, pace: bool) -> AtResult {
        let mut guard = self.stream_slot();
        if guard.is_none() {
            *guard = self.connect();
        }
        let stream = match guard.as_mut() {
            Some(s) => s,
            None => return AtResult::Transport,
        };
        if self.prime_and_send(stream, cmd, pace).is_err() {
            *guard = None;
            return AtResult::Transport;
        }
        let mut resp = Vec::with_capacity(2048);
        let start = Instant::now();
        let mut echo_off: Option<usize> = None;
        let mut saw_prefix = false;
        loop {
            if start.elapsed() >= total {
                break;
            }
            let _ = stream.set_read_timeout(Some(Duration::from_millis(150)));
            let mut chunk = [0u8; 1024];
            match stream.read(&mut chunk) {
                Ok(0) => {
                    *guard = None;
                    return AtResult::Transport;
                }
                Ok(n) => {
                    resp.extend_from_slice(&chunk[..n]);
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
                                return AtResult::Ok(s);
                            }
                            if tail.contains("ERROR") {
                                return AtResult::Err(s);
                            }
                        }
                    }
                }
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => {
                    *guard = None;
                    return AtResult::Transport;
                }
            }
        }
        AtResult::Ok(String::from_utf8_lossy(&resp).into_owned())
    }

    /// Paced, hardened exchange with one patient retry.
    pub fn at_expect(&self, cmd: &str, prefix: &str, total: Duration) -> AtResult {
        match self.at_full(cmd, prefix, total, true) {
            AtResult::Ok(s) if s.contains(prefix) => AtResult::Ok(s),
            AtResult::Ok(_) => {
                std::thread::sleep(Duration::from_millis(350));
                *self.stream_slot() = None;
                match self.at_full(cmd, prefix, total, true) {
                    AtResult::Ok(s2) if s2.contains(prefix) => AtResult::Ok(s2),
                    other => other,
                }
            }
            AtResult::Transport => self.at_full(cmd, prefix, total, true),
            other => other,
        }
    }

    /// APDU exchange over AT+CSIM (basic + logical channels pass through).
    pub fn transmit(&self, apdu: &[u8]) -> Result<Vec<u8>, ()> {
        if apdu.len() > 512 || apdu.is_empty() {
            return Ok(vec![0x6F, 0x00]);
        }
        let hex = at::bin2hex(apdu);
        let cmd = format!("AT+CSIM={},\"{}\"", apdu.len() * 2, hex);
        append_log(&format!("XMIT {}", hex));
        match self.at_expect(&cmd, "+CSIM:", Duration::from_millis(8000)) {
            AtResult::Ok(resp) => {
                let parsed = at::parse_csim(&resp);
                append_log(&format!("  RESP parsed={:?}", parsed.as_ref().map(|v| v.len())));
                parsed.ok_or(())
            }
            // Modem refused OR went silent after retries: surface a
            // card-level SW instead of killing the PC/SC session — tools
            // like pySim-shell probe unknown AIDs and must survive them.
            _ => Ok(vec![0x6F, 0x00]),
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
        // Unpaced: pcscd's status thread polls presence frequently.
        if let AtResult::Ok(resp) = self.at_full("AT+CPIN?", "+CPIN:", Duration::from_millis(2500), false) {
            if resp.contains("+CPIN:") {
                p = true;
                got = true;
            }
        }
        if !got {
            *self.stream_slot() = None;
            if let AtResult::Ok(r2) = self.at_full("AT+CPIN?", "+CPIN:", Duration::from_millis(2500), false) {
                p = r2.contains("+CPIN:");
            }
        }
        *self.presence_ts.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        *self.present.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
        p
    }

    /// Session init (lpac-style capability probe). Echo ON for correlation.
    pub fn init_handshake(&self) -> bool {
        matches!(self.at_full("ATE1", "OK", Duration::from_millis(2000), false), AtResult::Ok(_))
            && matches!(self.at_full("AT+CSIM=?", "OK", Duration::from_millis(2500), false), AtResult::Ok(_))
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
