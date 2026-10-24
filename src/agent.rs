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

pub struct AgentConfig {
    pub vpcd_addr: String,
    pub target: String,
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
        Frame::Control(vpcd::CTRL_ATR) | Frame::Control(vpcd::CTRL_RESET) => {
            wlog(log, "vpcd: ATR/RESET -> ATR");
            vpcd_sock.write_all(&vpcd::encode(atr))
        }
        Frame::Control(other) => {
            wlog(log, &format!("vpcd: unknown control 0x{other:02X} ignored"));
            Ok(())
        }
        Frame::Apdu(capdu) => {
            let rapdu = match reader.transmit(&capdu) {
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
            "modem-vpicc: vpcd={} target={} atr={}B reconnect={}ms",
            cfg.vpcd_addr,
            cfg.target,
            cfg.atr.len(),
            cfg.reconnect_ms
        ),
    );
    loop {
        match TcpStream::connect(&cfg.vpcd_addr) {
            Ok(mut sock) => {
                let reader = Reader::new(&cfg.target);
                if reader.init_handshake() {
                    wlog(&mut log, "modem-vpicc: connected, modem handshake ok");
                    if let Err(e) = serve_connection(&mut sock, &reader, &cfg.atr, &mut log) {
                        wlog(&mut log, &format!("modem-vpicc: session ended: {e}"));
                    }
                } else {
                    wlog(&mut log, "modem-vpicc: modem handshake failed; retrying");
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
