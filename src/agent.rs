//! modem-vpicc agent core: connects OUT to a vpcd listener (vsmartcard
//! virtual reader) and plays the smartcard backed by the modem SIM. The
//! modem side reuses `Reader`, so echo-correlated AT+CSIM, the pacing
//! floor and the strict parser behave identically to the pcscd driver
//! path. Wire facts and operational caveats: VPCD-ARCHITECTURE.md.

use crate::reader::Reader;
use crate::vpcd::{self, Frame};
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::time::Duration;

/// Hard cap on buffered vpcd input: a legit frame is at most 65537
/// bytes; anything past frame+chunk means a hostile or desynced peer.
const STREAM_CAP: usize = 1 << 20;

/// Every APDU must be answered within the vpcd driver's 2 s receive
/// window (libifdvpcd-amperstrand VPCD_RECV_TIMEOUT_SEC); a slower
/// answer returns empty to the host and ejects the VICC. 1.5 s keeps
/// ~4x margin over the modem's 400 ms p50 while landing inside it.
const APDU_TIMEOUT: Duration = Duration::from_millis(1500);

pub struct AgentConfig {
    pub vpcd_addr: String,
    /// One or more modem targets, tried in order every cycle (round 3:
    /// this modem's AT windows FLAP BETWEEN PORTS mid-session —
    /// ttyUSB2 dead while ttyUSB3 is alive — so the agent must
    /// re-detect the live port every handshake cycle, not hold one).
    /// Comma-separated in `--target`.
    pub targets: Vec<String>,
    pub atr: Vec<u8>,
    pub reconnect_ms: u64,
    pub log_path: Option<String>,
}

pub fn serve_connection(
    vpcd_sock: &mut TcpStream,
    reader: &Reader,
    atr: &[u8],
    log: &mut Option<File>,
) -> io::Result<()> {
    vpcd_sock.set_nodelay(true)?;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        while let Some((frame, used)) = vpcd::decode(&buf) {
            buf.drain(..used);
            handle_frame(vpcd_sock, reader, atr, frame, log)?;
        }
        if vpcd::expected(&buf) == Some(2) && buf.len() >= 2 {
            // zero-length frame: protocol noise from a desynced peer
            buf.drain(..2);
            continue;
        }
        if buf.len() > STREAM_CAP {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "vpcd stream cap exceeded",
            ));
        }
        let n = vpcd_sock.read(&mut chunk)?;
        if n == 0 {
            return Err(io::Error::new(ErrorKind::UnexpectedEof, "vpcd peer closed"));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn handle_frame(
    vpcd_sock: &mut TcpStream,
    reader: &Reader,
    atr: &[u8],
    frame: Frame,
    log: &mut Option<File>,
) -> io::Result<()> {
    match frame {
        Frame::Control(vpcd::CTRL_OFF) => {
            // fire-and-forget by protocol contract: an empty ack here
            // de-syncs the driver's next ATR probe (bench-verified)
            wlog(log, "vpcd: OFF (no reply)");
            Ok(())
        }
        Frame::Control(vpcd::CTRL_ON) => {
            // upstream vicc_poweron(): ON is send-only — the driver
            // NEVER reads a reply (verified: the only reads live in
            // vicc_transmit = getatr + APDUs; the amperstrand patch
            // does not change this; both bench-proven vpicc clients —
            // ESP32 vpcd.rs "fire-and-forget", VpcdBridge "OFF/ON: no
            // reply" — stay silent). Answering would leave a stale
            // frame the driver's NEXT read consumes one request late.
            wlog(log, "vpcd: ON (no reply)");
            Ok(())
        }
        Frame::Control(vpcd::CTRL_RESET) => {
            // upstream vicc_reset() is send-only too: IFDHPowerICC's
            // RESET case always follows with its OWN getatr (an ATR
            // control that IS read). Answering the RESET leaves a
            // stale frame that shifts every later read one request
            // late — invisible while all answers are ATRs, fatal the
            // moment a C-APDU's R-APDU gets replaced by the stale ATR.
            // (round 5; the proven clients answer RESET, but their
            // benches never exercise RESET-followed-by-transmit.)
            wlog(log, "vpcd: RESET (no reply — driver follows with its own ATR request)");
            Ok(())
        }
        Frame::Control(vpcd::CTRL_ATR) => {
            wlog(log, "vpcd: ATR request -> ATR");
            vpcd_sock.write_all(&vpcd::encode(atr))
        }
        Frame::Control(other) => {
            wlog(log, &format!("vpcd: unknown control 0x{other:02X} ignored"));
            Ok(())
        }
        Frame::Apdu(capdu) => {
            wlog(log, "vpcd: APDU -> modem");
            let rapdu = match reader.transmit_with_timeout(&capdu, APDU_TIMEOUT) {
                Ok(r) => r,
                Err(()) => {
                    // vpcd has no transport-error frame; refused/drop map
                    // to SW 6F00 exactly like the driver's card-level path
                    wlog(log, "modem transport error -> 6F00");
                    vec![0x6F, 0x00]
                }
            };
            vpcd_sock.write_all(&vpcd::encode(&rapdu))
        }
    }
}

pub fn run(cfg: AgentConfig) {
    let mut log = open_log(cfg.log_path.as_deref());
    wlog(
        &mut log,
        &format!(
            "modem-vpicc: vpcd={} targets=[{}] atr={}B reconnect={}ms",
            cfg.vpcd_addr,
            cfg.targets.join(", "),
            cfg.atr.len(),
            cfg.reconnect_ms
        ),
    );
    // One Reader per target, constructed once and surviving reconnects
    // (firmware-client #349 doctrine; also stops re-spawning stty per
    // reconnect on the router). Each cycle tries targets in order —
    // the first whose handshake succeeds is the one served, so a
    // mid-session window flap between ports is followed on the next
    // cycle instead of pinning a dead port.
    let readers: Vec<(String, Reader)> = cfg
        .targets
        .iter()
        .map(|t| (t.clone(), Reader::new(t)))
        .collect();
    // Live-state visibility (2026-11-21 live tests): this modem's AT
    // windows exist only ~minutes after a uqmi reset-mode poke, so
    // "handshake failing" is the NORMAL steady state for long
    // stretches. Log the first failure and then a slow heartbeat
    // (every 20th attempt ≈ 10 s at the 500 ms cadence) — carrying the
    // transport open report so live runs show exactly how far each
    // open got (stty result / device open / termios arm) — plus the
    // recovery line when a window opens.
    let mut handshake_fails: u32 = 0;
    loop {
        // MODEM FIRST, VPCD SECOND (2026-11-21 bench bug, see
        // docs/evidence/vpcd-handshake-debug/): the driver's presence
        // poll accepts this socket within ~400 ms of connect and starts
        // a 2 s answer window (libifdvpcd-amperstrand
        // VPCD_RECV_TIMEOUT_SEC). Doing the modem handshake BETWEEN
        // connect and the serve loop loses that window on a slow tty —
        // the driver ejects (host socket lingers FIN-WAIT-2), pcscd
        // never sees the card, and the agent reconnect-churns forever.
        // Handshake before connect; enter the serve loop the instant
        // connect returns; stay unconnected while the modem is down
        // (reader honestly absent instead of vpcd churn).
        let mut winner: Option<&(String, Reader)> = None;
        for entry in &readers {
            if entry.1.init_handshake() {
                winner = Some(entry);
                break;
            }
        }
        let Some((won_target, reader)) = winner else {
            handshake_fails += 1;
            if handshake_fails == 1 || handshake_fails.is_multiple_of(20) {
                let report = crate::transport::last_open_report()
                    .unwrap_or_else(|| "no open attempt".into());
                wlog(
                    &mut log,
                    &format!(
                        "modem-vpicc: no target answered (attempt {handshake_fails} over {} target(s)); not connecting to vpcd; last open: {report}",
                        readers.len()
                    ),
                );
            }
            std::thread::sleep(Duration::from_millis(cfg.reconnect_ms));
            continue;
        };
        if handshake_fails > 0 {
            wlog(
                &mut log,
                &format!(
                    "modem-vpicc: AT window opened on {won_target} after {handshake_fails} failed attempt(s)"
                ),
            );
            handshake_fails = 0;
        }
        match TcpStream::connect(&cfg.vpcd_addr) {
            Ok(mut sock) => {
                wlog(
                    &mut log,
                    &format!("modem-vpicc: connected via {won_target} (handshake done pre-connect)"),
                );
                if let Err(e) = serve_connection(&mut sock, reader, &cfg.atr, &mut log) {
                    wlog(&mut log, &format!("modem-vpicc: session ended: {e}"));
                }
            }
            Err(e) => wlog(&mut log, &format!("modem-vpicc: vpcd connect failed: {e}")),
        }
        std::thread::sleep(Duration::from_millis(cfg.reconnect_ms));
    }
}

/// Opt-in log file under the driver's v3.4.1 doctrine: 0600, never
/// through a symlink, fail closed when permissions cannot be secured.
fn open_log(path: Option<&str>) -> Option<File> {
    let path = path?;
    if std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        eprintln!("modem-vpicc: refusing symlinked log path {path}");
        return None;
    }
    let f = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .ok()?;
    if f.set_permissions(std::fs::Permissions::from_mode(0o600))
        .is_err()
    {
        eprintln!("modem-vpicc: cannot secure log permissions for {path}; logging disabled");
        return None;
    }
    Some(f)
}

fn wlog(log: &mut Option<File>, msg: &str) {
    if let Some(f) = log.as_mut() {
        let _ = writeln!(f, "{}", msg);
    }
}
