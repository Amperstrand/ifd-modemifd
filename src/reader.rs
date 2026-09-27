//! One Reader = one modem (one PC/SC slot). Transport: TCP to a bridge
//! (socat) on the modem host. AT semantics proven on the bench (2026-09-27):
//! drain -> send -> read until OK/ERROR -> retry-with-reconnect on silence.


fn append_log(msg: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("/tmp/ifd-modemifd-reader.log") {
        let _ = writeln!(f, "{}", msg);
    }
}

use crate::at;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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

    /// One AT exchange. Caller holds no other locks; internal locking on
    /// the stream only.
    fn at_once(&self, cmd: &str, total: Duration) -> AtResult {
        let mut guard = self.stream_slot();
        if guard.is_none() {
            *guard = self.connect();
        }
        let stream = match guard.as_mut() {
            Some(s) => s,
            None => return AtResult::Transport,
        };
        // drain pending bytes (URCs from the shared modem port)
        let _ = stream.set_nonblocking(true);
        let mut sink = [0u8; 2048];
        let deadline = Instant::now() + Duration::from_millis(120);
        while Instant::now() < deadline {
            match stream.read(&mut sink) {
                Ok(0) => break,
                Ok(_) => {}
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    *guard = None;
                    return AtResult::Transport;
                }
            }
        }
        let _ = stream.set_nonblocking(false);
        let line = format!("{}\r", cmd);
        if stream.write_all(line.as_bytes()).is_err() {
            *guard = None;
            return AtResult::Transport;
        }
        let mut resp = Vec::with_capacity(2048);
        let start = Instant::now();
        loop {
            let waited = start.elapsed();
            if waited >= total {
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
                    if s.contains("OK\r") {
                        return AtResult::Ok(s);
                    }
                    if s.contains("ERROR") {
                        return AtResult::Err(s);
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
        AtResult::Ok(String::from_utf8_lossy(&resp).into_owned()) // soft timeout
    }

    /// Hardened exchange: one retry on transport error or silent response.
    pub fn at(&self, cmd: &str, total: Duration) -> AtResult {
        let first = self.at_once(cmd, total);
        match first {
            AtResult::Transport => self.at_once(cmd, total),
            AtResult::Ok(ref s) if s.is_empty() => {
                *self.stream_slot() = None; // force reconnect
                let _ = self.at_once("AT", Duration::from_millis(1000));
                self.at_once(cmd, total)
            }
            other => other,
        }
    }

    /// Prefix-matched exchange: keeps reading (discarding unrelated URC
    /// traffic such as foreign +CPIN answers on the shared port) until a
    /// line starting with `prefix` is seen and the command completes, or
    /// the overall timeout expires. This is the robust pattern lpac uses.
    pub fn at_expect(&self, cmd: &str, prefix: &str, total: Duration) -> AtResult {
        match self.at_expect_once(cmd, prefix, total) {
            AtResult::Ok(s) => {
                if s.contains(prefix) {
                    AtResult::Ok(s)
                } else {
                    std::thread::sleep(Duration::from_millis(350));
                    *self.stream_slot() = None;
                    self.at_expect_once(cmd, prefix, total)
                }
            }
            AtResult::Transport => self.at_expect_once(cmd, prefix, total),
            other => other,
        }
    }

    fn at_expect_once(&self, cmd: &str, prefix: &str, total: Duration) -> AtResult {
        let mut guard = self.stream_slot();
        if guard.is_none() {
            *guard = self.connect();
        }
        let stream = match guard.as_mut() {
            Some(s) => s,
            None => return AtResult::Transport,
        };
        let _ = stream.set_nonblocking(true);
        let mut sink = [0u8; 2048];
        let deadline = Instant::now() + Duration::from_millis(120);
        while Instant::now() < deadline {
            match stream.read(&mut sink) {
                Ok(0) => break,
                Ok(_) => {}
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    *guard = None;
                    return AtResult::Transport;
                }
            }
        }
        let _ = stream.set_nonblocking(false);
        // Pacing: the RG502Q firmware thrashes SIM sessions when AT
        // commands arrive back-to-back (each triggers a +CPIN URC and can
        // swallow the real response). 250ms between commands is stable.
        {
            let mut lc = self.last_cmd.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(t) = *lc {
                let wait = Duration::from_millis(250).saturating_sub(t.elapsed());
                if !wait.is_zero() {
                    std::thread::sleep(wait);
                }
            }
            *lc = Some(Instant::now());
        }
        if stream.write_all(format!("{}\r", cmd).as_bytes()).is_err() {
            *guard = None;
            return AtResult::Transport;
        }
        let mut resp = Vec::with_capacity(2048);
        let start = Instant::now();
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
                    if !saw_prefix && s.contains(prefix) {
                        saw_prefix = true;
                    }
                    if saw_prefix {
                        // command completes at OK/ERROR after our prefix
                        if s.contains("OK\r") {
                            return AtResult::Ok(s);
                        }
                        if s.contains("ERROR") {
                            return AtResult::Err(s);
                        }
                    } else if s.contains("ERROR") && !s.contains(prefix) {
                        // modem rejected OUR command outright (before any prefix)
                        // heuristic: only trust this near the end
                        if start.elapsed() >= total - Duration::from_millis(600) {
                            return AtResult::Err(s);
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

    /// APDU exchange over AT+CSIM (basic + logical channels: raw MANAGE
    /// CHANNEL passes through on proven firmwares).
    pub fn transmit(&self, apdu: &[u8]) -> Result<Vec<u8>, ()> {
        if apdu.len() > 512 || apdu.is_empty() {
            return Ok(vec![0x6F, 0x00]);
        }
        let hex = at::bin2hex(apdu);
        let cmd = format!("AT+CSIM={},\"{}\"", apdu.len() * 2, hex);
        append_log(&format!("XMIT {} -> {:?}", hex, cmd));
        match self.at_expect(&cmd, "+CSIM:", Duration::from_millis(8000)) {
            AtResult::Ok(resp) => {
                append_log(&format!("  RESP {:?}", resp));
                at::parse_csim(&resp).ok_or(())
            }
            AtResult::Err(_) => Ok(vec![0x6F, 0x00]), // modem refused: map to SW 6F00
            AtResult::Transport => Err(()),
        }
    }

    pub fn presence(&self) -> bool {
        {
            let ts = self.presence_ts.lock().unwrap_or_else(|e| e.into_inner());
            let p = self.present.lock().unwrap_or_else(|e| e.into_inner());
            if let (Some(t), Some(v)) = (ts.as_ref(), p.as_ref()) {
                if t.elapsed() < Duration::from_secs(3) {
                    return *v;
                }
            }
        }
        let mut p = false;
        match self.at_expect("AT+CPIN?", "+CPIN:", Duration::from_millis(4000)) {
            AtResult::Ok(resp) => {
                p = at::parse_cpin_present(&resp);
                if !p {
                    *self.stream_slot() = None; // stale/contended: force fresh conn
                    if let AtResult::Ok(r2) = self.at_expect("AT+CPIN?", "+CPIN:", Duration::from_millis(4000)) {
                        p = at::parse_cpin_present(&r2);
                    }
                }
            }
            _ => {
                *self.stream_slot() = None;
            }
        }
        *self.presence_ts.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        *self.present.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
        p
    }

    /// Session init, modelled on lpac's connect probe.
    pub fn init_handshake(&self) -> bool {
        matches!(self.at("ATE0", Duration::from_millis(1500)), AtResult::Ok(_))
            && matches!(self.at("AT+CSIM=?", Duration::from_millis(2000)), AtResult::Ok(_))
    }

    pub fn close(&self) {
        *self.stream_slot() = None;
    }
}
