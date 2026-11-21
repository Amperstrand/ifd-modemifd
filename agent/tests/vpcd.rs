//! vpcd codec units + modem-vpicc agent integration battery. The
//! integration test stands up a mock modem (AT dialect, mock_at.rs
//! pattern) and a mock vpcd server, runs the real agent binary against
//! both, and asserts the wire contract from VPCD-ARCHITECTURE.md —
//! including the no-reply-for-OFF invariant and reconnect behavior.

use ifd_modemifd::consts::FAKE_ATR;
use ifd_modemifd::vpcd::{self, Frame};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

#[test]
fn codec_round_trip_apdu() {
    let payload = vec![0x00, 0xA4, 0x00, 0x0C, 0x02, 0x3F, 0x00];
    let wire = vpcd::encode(&payload);
    assert_eq!(wire, [0x00, 0x07, 0x00, 0xA4, 0x00, 0x0C, 0x02, 0x3F, 0x00]);
    let (frame, used) = vpcd::decode(&wire).unwrap();
    assert_eq!(frame, Frame::Apdu(payload));
    assert_eq!(used, wire.len());
}

#[test]
fn codec_controls_match_reference() {
    // values per upstream vpcd.h (pinned 93286a9): OFF=0, ON=1, RESET=2, ATR=4
    for (byte, name) in [
        (0x00u8, "OFF"),
        (0x01, "ON"),
        (0x02, "RESET"),
        (0x04, "ATR"),
    ] {
        let wire = vpcd::encode(&[byte]);
        let (frame, used) = vpcd::decode(&wire).unwrap();
        assert_eq!(frame, Frame::Control(byte), "{name}");
        assert_eq!(used, 3);
    }
}

#[test]
fn codec_partial_buffer_is_not_decoded() {
    let wire = vpcd::encode(&[0x00, 0xA4, 0x00]);
    assert_eq!(vpcd::expected(&wire[..1]), None);
    assert_eq!(vpcd::decode(&wire[..1]), None);
    assert_eq!(vpcd::expected(&wire), Some(wire.len()));
    assert_eq!(vpcd::decode(&wire[..wire.len() - 1]), None);
    assert!(vpcd::decode(&wire).is_some());
}

#[test]
fn codec_empty_payload_is_skippable_noise() {
    let wire = [0x00u8, 0x00];
    assert_eq!(vpcd::expected(&wire), Some(2));
    assert!(vpcd::decode(&wire).is_none());
}

struct MockModem {
    csim_delivered: Arc<AtomicUsize>,
}

impl MockModem {
    fn spawn() -> (MockModem, String) {
        MockModem::spawn_delays(0, 0)
    }

    /// AT-dialect modem mock with artificial pre-answer delays for the
    /// two `init_handshake` exchanges (`ATE1`, `AT+CSIM=?`). Each delay
    /// must stay under its exchange deadline (2 s / 2.5 s) to model a
    /// SLOW-BUT-SUCCESSFUL handshake — the NR7101 tty shape that lost
    /// the driver's 2 s ATR race on the bench (2026-11-21).
    fn spawn_delays(ate1_delay_ms: u64, csim_test_delay_ms: u64) -> (MockModem, String) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap().to_string();
        let csim = Arc::new(AtomicUsize::new(0));
        let csim_thread = Arc::clone(&csim);
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { break };
                let csim = Arc::clone(&csim_thread);
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    loop {
                        let mut b = [0u8; 1];
                        match conn.read(&mut b) {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {
                                if b[0] == b'\r' {
                                    let line = String::from_utf8_lossy(&buf).into_owned();
                                    buf.clear();
                                    if line.starts_with("AT+CSIM") {
                                        csim.fetch_add(1, Ordering::SeqCst);
                                    }
                                    let delay_ms = match line.as_str() {
                                        "ATE1" => ate1_delay_ms,
                                        "AT+CSIM=?" => csim_test_delay_ms,
                                        _ => 0,
                                    };
                                    if delay_ms > 0 {
                                        std::thread::sleep(Duration::from_millis(delay_ms));
                                    }
                                    reply_modem(&mut conn, &line);
                                } else {
                                    buf.push(b[0]);
                                }
                            }
                        }
                    }
                });
            }
        });
        (
            MockModem {
                csim_delivered: csim,
            },
            addr,
        )
    }
}

fn reply_modem(conn: &mut TcpStream, line: &str) {
    let _ = conn.write_all(format!("\r\n{line}\r\n").as_bytes());
    if line.starts_with("AT+CSIM=") {
        let _ = conn.write_all(b"\r\n+CSIM: 4,\"9000\"\r\n\r\nOK\r\n");
    } else if line == "AT+CPIN?" {
        let _ = conn.write_all(b"\r\n+CPIN: READY\r\n\r\n\r\nOK\r\n");
    } else {
        let _ = conn.write_all(b"\r\n\r\nOK\r\n");
    }
}

struct MockVpcd {
    listener: TcpListener,
    conn: Mutex<Option<TcpStream>>,
}

impl MockVpcd {
    fn spawn() -> (MockVpcd, String) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap().to_string();
        listener.set_nonblocking(true).unwrap();
        (
            MockVpcd {
                listener,
                conn: Mutex::new(None),
            },
            addr,
        )
    }

    fn accept_agent(&self, deadline: Duration) -> TcpStream {
        let start = Instant::now();
        loop {
            if let Ok((sock, _)) = self.listener.accept() {
                sock.set_nonblocking(false).unwrap();
                sock.set_read_timeout(Some(Duration::from_millis(2500)))
                    .unwrap();
                *self.conn.lock().unwrap() = Some(sock.try_clone().unwrap());
                return sock;
            }
            assert!(
                start.elapsed() < deadline,
                "agent never connected to mock vpcd"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn send(&self, payload: &[u8]) {
        self.conn
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .write_all(&vpcd::encode(payload))
            .unwrap();
    }

    /// Write pre-encoded bytes verbatim (multi-frame single-write —
    /// the shape pcscd produces when POWER_UP sends back-to-back
    /// controls in one syscall burst).
    fn send_raw(&self, bytes: &[u8]) {
        self.conn
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }

    fn read_frame(&self) -> Option<Vec<u8>> {
        let mut conn = self.conn.lock().unwrap().take().unwrap();
        let frame = read_frame_blocking(&mut conn);
        *self.conn.lock().unwrap() = Some(conn);
        frame
    }

    fn read_silence(&self, within: Duration) -> bool {
        let mut conn = self.conn.lock().unwrap().take().unwrap();
        conn.set_read_timeout(Some(within)).unwrap();
        let mut b = [0u8; 1];
        let got = conn.read(&mut b);
        conn.set_read_timeout(Some(Duration::from_millis(2500)))
            .unwrap();
        *self.conn.lock().unwrap() = Some(conn);
        match got {
            Err(e) => matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ),
            _ => false,
        }
    }
}

fn read_frame_blocking(conn: &mut TcpStream) -> Option<Vec<u8>> {
    let mut hdr = [0u8; 2];
    let mut filled = 0;
    while filled < 2 {
        match conn.read(&mut hdr[filled..]) {
            Ok(0) => return None,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return None,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
    let len = u16::from_be_bytes(hdr) as usize;
    let mut payload = vec![0u8; len];
    let mut filled = 0;
    while filled < len {
        match conn.read(&mut payload[filled..]) {
            Ok(0) => return None,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return None,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
    Some(payload)
}

fn spawn_agent(vpcd_addr: &str, modem_addr: &str) -> Child {
    Command::new(env!("CARGO_BIN_EXE_modem-vpicc"))
        .args([
            "--vpcd",
            vpcd_addr,
            "--target",
            &format!("tcp://{modem_addr}"),
            "--reconnect-ms",
            "100",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn modem-vpicc")
}

#[test]
fn agent_speaks_the_vpcd_contract() {
    let (modem, modem_addr) = MockModem::spawn();
    let (vpcd_srv, vpcd_addr) = MockVpcd::spawn();
    let mut agent = spawn_agent(&vpcd_addr, &modem_addr);

    let sock = vpcd_srv.accept_agent(Duration::from_secs(10));

    vpcd_srv.send(&[vpcd::CTRL_ATR]);
    let atr = vpcd_srv.read_frame().expect("ATR frame after ATR control");
    assert_eq!(
        atr,
        FAKE_ATR.to_vec(),
        "ATR must match the driver's fake ATR for parity"
    );

    vpcd_srv.send(&[0x00, 0xA4, 0x00, 0x0C, 0x02, 0x3F, 0x00]);
    let rapdu = vpcd_srv.read_frame().expect("R-APDU frame");
    assert_eq!(
        rapdu,
        vec![0x90, 0x00],
        "canned modem answers 9000: {}",
        hex(&rapdu)
    );

    vpcd_srv.send(&[vpcd::CTRL_OFF]);
    assert!(
        vpcd_srv.read_silence(Duration::from_millis(300)),
        "OFF must get NO reply"
    );

    // round 5: upstream vicc_reset() is send-only — the driver always
    // follows a RESET with its OWN ATR request. Answering the RESET
    // leaves a stale frame that shifts every later read one request
    // late (an R-APDU would be replaced by the stale ATR).
    vpcd_srv.send(&[vpcd::CTRL_RESET]);
    assert!(
        vpcd_srv.read_silence(Duration::from_millis(300)),
        "RESET must get NO reply (the driver reads its next answer after its own ATR request)"
    );
    vpcd_srv.send(&[vpcd::CTRL_ATR]);
    let atr2 = vpcd_srv.read_frame().expect("ATR frame after ATR request");
    assert_eq!(atr2, FAKE_ATR.to_vec());

    drop(sock);
    *vpcd_srv.conn.lock().unwrap() = None;
    let _reconnected = vpcd_srv.accept_agent(Duration::from_secs(5));
    vpcd_srv.send(&[vpcd::CTRL_ATR]);
    let atr3 = vpcd_srv.read_frame().expect("ATR after reconnect");
    assert_eq!(atr3, FAKE_ATR.to_vec(), "agent must reconnect and re-serve");

    assert!(
        modem.csim_delivered.load(Ordering::SeqCst) >= 2,
        "APDUs reached the modem"
    );
    let _ = agent.kill();
    let _ = agent.wait();
}

// ---------------------------------------------------------------------------
// DriverMock — the amperstrand-patched libifdvpcd listener as a machine
// ---------------------------------------------------------------------------

/// `VPCD_RECV_TIMEOUT_SEC` from libifdvpcd-amperstrand.patch: every
/// vpicc answer must land within this window of the request or the
/// socket is ejected (upstream recvall would block forever — the
/// pcscd-EH wedge the patch fixed).
const DRIVER_WINDOW: Duration = Duration::from_millis(2000);

/// pcscd polls `IFDHICCPresence` at `PCSCLITE_STATUS_POLL_RATE` ≈ 400 ms;
/// each poll is `vicc_connect` (accept-on-demand) + `vicc_getatr`
/// (ATR control + one bounded answer). The mock shortens the cadence to
/// keep tests fast — the race SHAPE (accept happens after connect, the
/// probe immediately after accept) is what must be modeled.
const DRIVER_POLL_MS: u64 = 100;

#[derive(Default)]
struct DriverEvents {
    /// completed accept()s (= distinct vpcd connections the agent made)
    accepts: AtomicUsize,
    /// per-probe answer latency in ms (probe send -> full ATR frame)
    answered: Mutex<Vec<u64>>,
    /// 2 s window expired -> vicc_eject (close) — the bench symptom
    ejects: AtomicUsize,
    /// vpicc closed on us (EOF/RST) before answering
    peer_closed: AtomicUsize,
}

impl DriverEvents {
    fn summary(&self) -> String {
        let answered = self.answered.lock().unwrap().clone();
        format!(
            "accepts={} answered={} (latencies {:?} ms) ejects={} peer_closed={}",
            self.accepts.load(Ordering::SeqCst),
            answered.len(),
            answered,
            self.ejects.load(Ordering::SeqCst),
            self.peer_closed.load(Ordering::SeqCst),
        )
    }
}

struct DriverMock {
    events: Arc<DriverEvents>,
}

enum ProbeOutcome {
    Answered(Duration),
    Timeout,
    Closed,
}

impl DriverMock {
    /// Spawn the patched-driver machine: poll-cadence accept, ATR
    /// control on accept, one answer per probe bounded by
    /// `DRIVER_WINDOW`, eject on expiry; the listener persists across
    /// ejections exactly like vicc_eject (client closed, server kept).
    fn spawn() -> (DriverMock, String) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap().to_string();
        listener.set_nonblocking(true).unwrap();
        let events = Arc::new(DriverEvents::default());
        let ev = Arc::clone(&events);
        std::thread::spawn(move || loop {
            // vicc_present: accept a pending vpicc if we have no client
            let mut conn: Option<TcpStream> = None;
            while conn.is_none() {
                match listener.accept() {
                    Ok((sock, _)) => {
                        let _ = sock.set_nodelay(true);
                        conn = Some(sock);
                        ev.accepts.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => return,
                }
                std::thread::sleep(Duration::from_millis(DRIVER_POLL_MS));
            }
            // presence-poll loop on the accepted client
            if let Some(sock) = conn.as_mut() {
                loop {
                    // vicc_getatr: ATR control, one bounded answer
                    if sock.write_all(&vpcd::encode(&[vpcd::CTRL_ATR])).is_err() {
                        ev.peer_closed.fetch_add(1, Ordering::SeqCst);
                        break;
                    }
                    let t0 = Instant::now();
                    match read_frame_deadline(sock, t0, DRIVER_WINDOW) {
                        ProbeOutcome::Answered(latency) => {
                            ev.answered.lock().unwrap().push(latency.as_millis() as u64);
                        }
                        ProbeOutcome::Timeout => {
                            // patched vicc_transmit: r < 0 -> vicc_eject
                            ev.ejects.fetch_add(1, Ordering::SeqCst);
                            break;
                        }
                        ProbeOutcome::Closed => {
                            ev.peer_closed.fetch_add(1, Ordering::SeqCst);
                            break;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(DRIVER_POLL_MS));
                }
            }
            // ejected/closed: client socket dropped, listener persists
            std::thread::sleep(Duration::from_millis(DRIVER_POLL_MS));
        });
        (DriverMock { events }, addr)
    }
}

/// Read one vpcd frame under an absolute deadline (models the patched
/// driver's SO_RCVTIMEO-bounded recvall). `t0` is the probe-send
/// instant; the returned latency mirrors what the driver experiences.
fn read_frame_deadline(conn: &mut TcpStream, t0: Instant, window: Duration) -> ProbeOutcome {
    let deadline = t0 + window;
    let mut hdr = [0u8; 2];
    let mut filled = 0;
    while filled < 2 {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return ProbeOutcome::Timeout;
        };
        let _ = conn.set_read_timeout(Some(remaining.max(Duration::from_millis(1))));
        match conn.read(&mut hdr[filled..]) {
            Ok(0) => return ProbeOutcome::Closed,
            Ok(n) => filled += n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if Instant::now() >= deadline {
                    return ProbeOutcome::Timeout;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return ProbeOutcome::Closed,
        }
    }
    let len = u16::from_be_bytes(hdr) as usize;
    let mut payload = vec![0u8; len];
    let mut got = 0;
    while got < len {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return ProbeOutcome::Timeout;
        };
        let _ = conn.set_read_timeout(Some(remaining.max(Duration::from_millis(1))));
        match conn.read(&mut payload[got..]) {
            Ok(0) => return ProbeOutcome::Closed,
            Ok(n) => got += n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if Instant::now() >= deadline {
                    return ProbeOutcome::Timeout;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return ProbeOutcome::Closed,
        }
    }
    ProbeOutcome::Answered(t0.elapsed())
}

/// Regression — the 2026-11-21 bench bug (docs/evidence/
/// vpcd-handshake-debug/): the agent used to run its modem handshake
/// BETWEEN the vpcd connect and the serve loop. The patched driver
/// accepts within one presence-poll cadence of connect and gives the
/// ATR answer a 2 s window; whenever the handshake outran the window
/// the driver ejected every cycle (host FIN-WAIT-2, pcscd "no smart
/// card", silent reconnect churn). With the handshake done BEFORE the
/// connect, a ~3 s handshake cannot lose the race: the first probe is
/// answered in milliseconds and the connection stays up.
#[test]
fn slow_modem_handshake_cannot_lose_the_atr_race() {
    // 1.5 s + 1.5 s ≈ 3 s handshake: each exchange inside its at_full
    // deadline (2 s / 2.5 s), together past the driver's 2 s window.
    let (_modem, modem_addr) = MockModem::spawn_delays(1500, 1500);
    let (driver, vpcd_addr) = DriverMock::spawn();
    let mut agent = spawn_agent(&vpcd_addr, &modem_addr);

    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let answered = driver.events.answered.lock().unwrap().len();
        if answered >= 3 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "driver never saw 3 answered probes — {}",
            driver.events.summary()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // quiescence: one full reconnect interval — a returned bug would
    // produce an eject + a second accept in this window
    std::thread::sleep(Duration::from_millis(400));

    let events = driver.events.summary();
    assert_eq!(
        driver.events.ejects.load(Ordering::SeqCst),
        0,
        "the patched driver must never eject — {events}"
    );
    assert_eq!(
        driver.events.accepts.load(Ordering::SeqCst),
        1,
        "the agent must hold ONE connection (no churn) — {events}"
    );
    for lat in driver.events.answered.lock().unwrap().iter() {
        assert!(
            *lat < DRIVER_WINDOW.as_millis() as u64,
            "probe answered outside the driver's 2 s window ({lat} ms) — {events}"
        );
    }
    let _ = agent.kill();
    let _ = agent.wait();
}

/// The same driver machine with an instant modem: the plain contract
/// (probe answered, connection stable, zero ejects) — controls the
/// mock against the pre-existing basic battery.
#[test]
fn agent_survives_the_patched_driver_machine() {
    let (_modem, modem_addr) = MockModem::spawn();
    let (driver, vpcd_addr) = DriverMock::spawn();
    let mut agent = spawn_agent(&vpcd_addr, &modem_addr);

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let answered = driver.events.answered.lock().unwrap().len();
        if answered >= 5 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "driver never saw 5 answered probes — {}",
            driver.events.summary()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(400));

    let events = driver.events.summary();
    assert_eq!(
        driver.events.ejects.load(Ordering::SeqCst),
        0,
        "unexpected eject — {events}"
    );
    assert_eq!(
        driver.events.accepts.load(Ordering::SeqCst),
        1,
        "unexpected reconnect churn — {events}"
    );
    let _ = agent.kill();
    let _ = agent.wait();
}

// ---------------------------------------------------------------------------
// tty-path regressions — the 2026-11-21 live hang (deployed sha 7e979f45)
// ---------------------------------------------------------------------------

/// Allocate a pty pair; returns (master, slave_path). The master side
/// is the scripted "modem", the slave is what the agent opens via
/// `tty:` — the only faithful local double for a serial AT port (TCP
/// mocks honor socket read timeouts and so cannot catch this class).
fn open_pty() -> (std::fs::File, String) {
    use std::os::unix::io::AsRawFd;
    let master = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/ptmx")
        .expect("open /dev/ptmx");
    let fd = master.as_raw_fd();
    let mut unlock: libc::c_int = 0;
    let rc = unsafe { libc::ioctl(fd, libc::TIOCSPTLCK, &mut unlock) };
    assert_eq!(rc, 0, "TIOCSPTLCK unlock");
    let mut buf = [0u8; 64];
    let rc = unsafe {
        libc::ptsname_r(fd, buf.as_mut_ptr() as *mut libc::c_char, buf.len())
    };
    assert_eq!(rc, 0, "ptsname_r");
    let len = buf.iter().position(|&b| b == 0).unwrap();
    let pts = std::str::from_utf8(&buf[..len]).unwrap().to_string();
    (master, pts)
}

/// Scripted pty modem: reads AT lines off the master and answers the
/// agent dialect (echo + result + OK) — but only while the "AT window"
/// is open. A closed window CONSUMES lines and answers nothing, which
/// is exactly the NR7101 after its post-reset AT window dies (uqmi
/// operating-mode windows last ~minutes; the live-test hang).
/// `close_after_exchanges` models the window dying MID-session: after
/// that many answered exchanges the daemon goes silent on its own.
struct PtyModem {
    window_open: Arc<AtomicBool>,
    exchanges: Arc<AtomicUsize>,
}

impl PtyModem {
    fn spawn(close_after_exchanges: Option<usize>) -> (PtyModem, String) {
        let (master, pts) = open_pty();
        let window_open = Arc::new(AtomicBool::new(true));
        let exchanges = Arc::new(AtomicUsize::new(0));
        let flag = Arc::clone(&window_open);
        let count = Arc::clone(&exchanges);
        std::thread::spawn(move || {
            let mut master = master;
            let mut buf = Vec::new();
            loop {
                let mut b = [0u8; 1];
                use std::io::Read;
                match master.read(&mut b) {
                    Ok(n) if n > 0 => {}
                    // 0/Err = no slave open right now (each failed
                    // handshake drops the agent's serial fd; a real USB
                    // tty survives this — a pty master sees EOF/EIO
                    // until the slave is reopened). Stay alive: the
                    // next attempt reopens the line.
                    _ => {
                        buf.clear();
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                }
                if b[0] == b'\r' {
                    let line = String::from_utf8_lossy(&buf).into_owned();
                    buf.clear();
                    if line.is_empty() {
                        continue;
                    }
                    if !flag.load(Ordering::SeqCst) {
                        continue; // AT window dead: consume, answer nothing
                    }
                    let n = count.fetch_add(1, Ordering::SeqCst) + 1;
                    // one-shot: die exactly AT the limit exchange, never
                    // re-arm (the test reopens the window explicitly)
                    if let Some(limit) = close_after_exchanges {
                        if n == limit {
                            flag.store(false, Ordering::SeqCst); // window dies now
                        }
                    }
                    use std::io::Write;
                    let _ = master.write_all(format!("\r\n{line}\r\n").as_bytes()); // echo
                    let body = if line.starts_with("AT+CSIM=") && line != "AT+CSIM=?" {
                        "\r\n+CSIM: 4,\"9000\"\r\n"
                    } else if line == "AT+CPIN?" {
                        "\r\n+CPIN: READY\r\n"
                    } else {
                        ""
                    };
                    let _ = master.write_all(body.as_bytes());
                    let _ = master.write_all(b"\r\n\r\nOK\r\n");
                } else {
                    buf.push(b[0]);
                }
            }
        });
        (
            PtyModem {
                window_open,
                exchanges,
            },
            pts,
        )
    }
}

/// Transport-level regression (the hang itself): a serial read under a
/// set deadline must return WouldBlock when the deadline fires — never
/// block in the kernel. On the live router the deployed agent (sha
/// 7e979f45) sat silent forever here: `SerialLink::set_read_timeout`
/// was a no-op, so `at_full`'s blocking tty read never returned to
/// check its deadline once the AT window died mid-handshake.
#[test]
fn tty_read_deadline_is_enforced() {
    let (_master_guard, pts) = open_pty(); // master stays open in scope
    let mut link = ifd_modemifd::transport::open(&format!("tty:{pts}")).expect("serial open");

    link.set_read_timeout(Some(Duration::from_millis(150)))
        .expect("arm serial read deadline");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut b = [0u8; 8];
        let _ = tx.send(link.read(&mut b));
    });
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {} // deadline fired
        Ok(other) => panic!("expected deadline WouldBlock, got {other:?}"),
        Err(_) => panic!(
            "tty read never returned — unbounded kernel block (the 7e979f45 live hang)"
        ),
    }

    // data present under an armed deadline still comes through
    let (master, pts2) = open_pty();
    let mut link2 = ifd_modemifd::transport::open(&format!("tty:{pts2}")).expect("serial open");
    link2.set_read_timeout(Some(Duration::from_millis(150))).unwrap();
    let mut m = master;
    use std::io::Write;
    m.write_all(b"OK\r").unwrap();
    let mut b = [0u8; 8];
    let n = link2.read(&mut b).expect("data read under armed deadline");
    assert_eq!(&b[..n], b"OK\r");
}

/// Reader-level regression: the AT window dies MID-handshake (ATE1
/// answered, AT+CSIM=? gets silence) — `init_handshake` must fail
/// bounded (per-exchange deadlines, not a kernel hang), and the next
/// attempt once the window reopens must succeed. This is the
/// production steady state on this modem: fail fast while AT-dead,
/// catch the next window.
#[test]
fn tty_window_death_mid_handshake_is_bounded_and_recovers() {
    // the window dies after the FIRST answered exchange: ATE1 gets its
    // echo+OK, AT+CSIM=? gets silence (the daemon consumed it)
    let (modem, pts) = PtyModem::spawn(Some(1));

    // bounded failure: deadlines are 2 s (ATE1) + 2.5 s (CSIM=?) + one
    // ~200 ms read overshoot each — anything past 8 s is a kernel hang
    let (tx, rx) = std::sync::mpsc::channel();
    let pts1 = pts.clone();
    std::thread::spawn(move || {
        let _ = tx.send(reader_init_handshake_bounded(&pts1));
    });
    let handshake1 = rx
        .recv_timeout(Duration::from_secs(8))
        .expect("init_handshake did not return — unbounded hang (the 7e979f45 live bug)");
    assert!(!handshake1, "handshake must fail while the window is dead");

    // window reopens: the very next attempt succeeds (the agent's
    // retry loop catches the next window)
    modem.window_open.store(true, Ordering::SeqCst);
    let (tx2, rx2) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx2.send(reader_init_handshake_bounded(&pts));
    });
    let handshake2 = rx2
        .recv_timeout(Duration::from_secs(8))
        .expect("recovery handshake did not return");
    assert!(handshake2, "handshake must succeed once the window reopens");
}

/// Round-5 regression — the FULL pcscd client-connect wire shape, and
/// the one-deep-skew class. pcscd's POWER_UP sends controls
/// back-to-back in one burst (strace-verified shape on this very
/// driver: ccid-firmware-rs TRANSMIT_DEAD_WINDOW.md — [OFF?][ON][ATR]
/// and IFD_RESET = [RESET][ATR], where ONLY the ATR request is ever
/// read). The agent must answer exactly once per read-bearing request
/// and never emit a frame for OFF/ON/RESET — any extra frame sits in
/// the driver's socket queue and shifts every subsequent read one
/// request late (an R-APDU silently replaced by a stale ATR).
#[test]
fn pcscd_activation_sequence_stays_frame_aligned() {
    let (_modem, modem_addr) = MockModem::spawn();
    let (vpcd_srv, vpcd_addr) = MockVpcd::spawn();
    let mut agent = spawn_agent(&vpcd_addr, &modem_addr);
    let _sock = vpcd_srv.accept_agent(Duration::from_secs(10));

    // pcscd POWER_UP burst: [OFF][ON][ATR] in ONE write.
    let mut burst = Vec::new();
    burst.extend_from_slice(&vpcd::encode(&[vpcd::CTRL_OFF]));
    burst.extend_from_slice(&vpcd::encode(&[vpcd::CTRL_ON]));
    burst.extend_from_slice(&vpcd::encode(&[vpcd::CTRL_ATR]));
    vpcd_srv.send_raw(&burst);
    let atr = vpcd_srv.read_frame().expect("exactly one ATR after the burst");
    assert_eq!(atr, FAKE_ATR.to_vec());
    assert!(
        vpcd_srv.read_silence(Duration::from_millis(300)),
        "OFF and ON must produce NO frames — any stale frame desyncs the driver"
    );

    // IFD_RESET shape: [RESET][ATR] in ONE write — same rule.
    let mut burst2 = Vec::new();
    burst2.extend_from_slice(&vpcd::encode(&[vpcd::CTRL_RESET]));
    burst2.extend_from_slice(&vpcd::encode(&[vpcd::CTRL_ATR]));
    vpcd_srv.send_raw(&burst2);
    let atr2 = vpcd_srv.read_frame().expect("exactly one ATR after RESET+ATR");
    assert_eq!(atr2, FAKE_ATR.to_vec());
    assert!(
        vpcd_srv.read_silence(Duration::from_millis(300)),
        "RESET must produce NO frame — the driver reads only after its own ATR request"
    );

    // alignment proof: the next APDU's answer must be THE APDU's
    // answer, not a stale ATR frame (the observable skew symptom)
    vpcd_srv.send(&[0x00, 0xA4, 0x00, 0x0C, 0x02, 0x3F, 0x00]);
    let rapdu = vpcd_srv.read_frame().expect("R-APDU frame");
    assert_eq!(
        rapdu,
        vec![0x90, 0x00],
        "R-APDU must be the modem's answer — a stale ATR here is the one-deep skew"
    );
    assert!(vpcd_srv.read_silence(Duration::from_millis(300)));

    let _ = agent.kill();
    let _ = agent.wait();
}

/// Run one `init_handshake` against a tty target on a worker thread
/// (the caller bounds it with `recv_timeout`; an unbounded kernel
/// block inside the handshake then fails the caller instead of
/// hanging the whole suite).
fn reader_init_handshake_bounded(pts: &str) -> bool {
    let reader = ifd_modemifd::reader::Reader::new(&format!("tty:{pts}"));
    reader.init_handshake()
}

/// Round-4 regression — THE root cause of every live stall: the
/// deployed OpenWrt router has NO stty binary, and the old open path
/// spawned one, so the tty stayed in CANONICAL mode (VMIN/VTIME
/// ignored under ICANON → blocking reads; kernel ECHO garbling the
/// link) and the handshake could never succeed regardless of AT
/// windows. The tty path must configure termios NATIVELY and never
/// spawn a child process.
#[test]
fn tty_configured_natively_without_stty() {
    use std::os::unix::io::AsRawFd;

    let (master, pts) = open_pty();

    // booby-trap: a PATH containing ONLY an stty that records its
    // execution and fails. If the transport ever spawns stty again,
    // the marker appears and this test fails.
    // trap dir lives under the workspace target/ (gitignored, on-disk):
    // /tmp is a saturated tmpfs on this bench (quota errors)
    let trap_dir =
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../target")).join(format!(
            "stty-trap-{}",
            std::process::id()
        ));
    let _ = std::fs::remove_dir_all(&trap_dir);
    std::fs::create_dir_all(&trap_dir).expect("trap dir");
    // builtin-only recorder: `: >` needs no PATH (a trap that calls
    // touch would silently fail under the poisoned PATH)
    let trap = "#!/bin/sh\n: > \"$0.marker\"\nexit 1\n";
    std::fs::write(trap_dir.join("stty"), trap).expect("trap stty");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(trap_dir.join("stty"), std::fs::Permissions::from_mode(0o755));
    }

    let old_path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", &trap_dir);
    let link = ifd_modemifd::transport::open(&format!("tty:{pts}"));
    std::env::set_var("PATH", &old_path);

    assert!(
        link.is_some(),
        "tty open must succeed with NO usable stty on PATH"
    );
    assert!(
        !trap_dir.join("stty.marker").exists(),
        "the tty path spawned stty — native termios is required (no stty on OpenWrt)"
    );
    let report = ifd_modemifd::transport::last_open_report().unwrap_or_default();
    assert!(
        report.contains("native termios ok"),
        "open report must show native termios config: {report}"
    );
    drop(link);

    // prove the slave is actually RAW now: fresh open + tcgetattr.
    // A fresh pty starts cooked (ICANON|ECHO) — only our native
    // configure could have flipped it.
    let probe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&pts)
        .expect("probe open");
    let mut tio: libc::termios = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { libc::tcgetattr(probe.as_raw_fd(), &mut tio) },
        0,
        "tcgetattr on probe"
    );
    assert_eq!(tio.c_lflag & libc::ICANON, 0, "canonical mode must be OFF");
    assert_eq!(tio.c_lflag & libc::ECHO, 0, "kernel echo must be OFF");
    assert_eq!(tio.c_iflag & libc::ICRNL, 0, "CR->NL translation OFF");
    assert_eq!(tio.c_oflag & libc::OPOST, 0, "output post-processing OFF");
    assert_eq!(tio.c_cflag & libc::CSIZE, libc::CS8, "8-bit chars");
    assert_eq!(tio.c_cc[libc::VMIN], 1, "VMIN=1 base state");
    assert_eq!(tio.c_cc[libc::VTIME], 0, "VTIME=0 base state");
    assert_eq!(
        unsafe { libc::cfgetispeed(&tio) },
        libc::B115200,
        "baud pinned to 115200"
    );
    drop(probe);
    drop(master); // keep the pty pair alive until assertions are done
    let _ = std::fs::remove_dir_all(&trap_dir);
}

/// Round-3 regression (deployed sha a8e772da hung with Threads:1 in a
/// blocking serial read despite the VMIN/VTIME fix): the deadline
/// primitive must be termios-INDEPENDENT. Sabotage the link into
/// blocking termios (VMIN=1/VTIME=0 — read() would block forever on a
/// silent port) and prove `wait_readable` still returns bounded.
/// This is the exact invariant whose violation hung the hardware and
/// which no pty could reproduce through the old read path.
#[test]
fn wait_readable_bounded_with_blocking_termios_armed() {
    let (_master, pts) = open_pty(); // silent "modem": no data ever
    let mut link = ifd_modemifd::transport::open(&format!("tty:{pts}")).expect("serial open");
    // sabotage: restore the blocking VMIN=1/VTIME=0 open-time setup —
    // models a USB tty where the VTIME arm does not take effect
    link.set_read_timeout(None).expect("sabotage: blocking termios");

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(link.wait_readable(Some(Duration::from_millis(300))));
    });
    let t0 = Instant::now();
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(false)) => {
            assert!(t0.elapsed() >= Duration::from_millis(250), "returned suspiciously fast");
        }
        Ok(other) => panic!("expected Ok(false) timeout, got {other:?}"),
        Err(_) => panic!(
            "wait_readable never returned with blocking termios armed — \
             the a8e772da hang class"
        ),
    }
}

/// Round-3 regression: the AT window dies MID-READ — the daemon echoes
/// the command, starts the response, and goes silent before the final
/// result code. The exchange must still fail bounded (deadline, not a
/// kernel block), and the retry loop must catch a reopened window.
#[test]
fn tty_window_death_mid_read_is_bounded() {
    let (master, pts) = open_pty();
    // daemon: answer exchange 1 fully; on exchange 2 send echo + one
    // stray byte of the response, then die
    std::thread::spawn(move || {
        let mut master = master;
        let mut buf = Vec::new();
        let mut exchanges = 0u32;
        loop {
            let mut b = [0u8; 1];
            use std::io::Read;
            match master.read(&mut b) {
                Ok(n) if n > 0 => {}
                _ => {
                    buf.clear();
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
            }
            if b[0] != b'\r' {
                buf.push(b[0]);
                continue;
            }
            let line = String::from_utf8_lossy(&buf).into_owned();
            buf.clear();
            if line.is_empty() {
                continue;
            }
            exchanges += 1;
            use std::io::Write;
            if exchanges == 1 {
                let _ = master.write_all(format!("\r\n{line}\r\n").as_bytes());
                let _ = master.write_all(b"\r\n\r\nOK\r\n");
            } else {
                // echo + the beginning of a response, then the window dies
                let _ = master.write_all(format!("\r\n{line}\r\n").as_bytes());
                let _ = master.write_all(b"\r\n\r\nO");
                return; // daemon gone; the stray byte is all there is
            }
        }
    });

    let (tx, rx) = std::sync::mpsc::channel();
    let target = format!("tty:{pts}");
    std::thread::spawn(move || {
        let reader = ifd_modemifd::reader::Reader::new(&target);
        let _ = tx.send(reader.init_handshake());
    });
    let got = rx
        .recv_timeout(Duration::from_secs(8))
        .expect("init_handshake never returned — mid-read window death hung it (a8e772da class)");
    assert!(!got, "handshake must fail bounded when the window dies mid-read");
}

/// Round-3 feature regression — this modem's AT windows FLAP BETWEEN
/// PORTS mid-session: `--target a,b` must fall through a dead first
/// target and serve through the live second (here: refused TCP +
/// a live pty modem).
#[test]
fn multi_target_falls_through_dead_port_to_live_one() {
    let (_modem, live_pts) = PtyModem::spawn(None); // window open
    let (driver, vpcd_addr) = DriverMock::spawn();
    let mut agent = Command::new(env!("CARGO_BIN_EXE_modem-vpicc"))
        .args([
            "--vpcd",
            &vpcd_addr,
            "--target",
            &format!("tcp://127.0.0.1:1,tty:{live_pts}"),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn modem-vpicc");

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let answered = driver.events.answered.lock().unwrap().len();
        if answered >= 3 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "agent never served through the live second target — {}",
            driver.events.summary()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(300));
    let events = driver.events.summary();
    assert_eq!(
        driver.events.ejects.load(Ordering::SeqCst),
        0,
        "no ejects — {events}"
    );
    assert_eq!(
        driver.events.accepts.load(Ordering::SeqCst),
        1,
        "one connection via the live target — {events}"
    );
    let _ = agent.kill();
    let _ = agent.wait();
}

/// Agent-level regression — the full production steady state through
/// the REAL binary on a REAL tty: AT window closed at boot (handshake
/// cycles bounded, agent stays OFF the vpcd listener), window opens
/// (uqmi-reset class), agent connects, first ATR probe answered inside
/// the driver's 2 s window, connection stable.
#[test]
fn agent_rides_tty_windows_with_real_binary() {
    let (modem, pts) = PtyModem::spawn(None);
    modem.window_open.store(false, Ordering::SeqCst); // window closed at boot
    let (driver, vpcd_addr) = DriverMock::spawn();

    let mut agent = Command::new(env!("CARGO_BIN_EXE_modem-vpicc"))
        .args([
            "--vpcd",
            &vpcd_addr,
            "--target",
            &format!("tty:{pts}"),
            "--reconnect-ms",
            "150",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn modem-vpicc");

    // while the window is shut: bounded handshake failures, and the
    // agent must NOT touch the vpcd listener (health gate)
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        driver.events.accepts.load(Ordering::SeqCst),
        0,
        "agent connected to vpcd while its modem was AT-dead — {}",
        driver.events.summary()
    );

    // window opens: connect + serve, first probe answered in-window
    modem.window_open.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let answered = driver.events.answered.lock().unwrap().len();
        if answered >= 3 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "window opened but the agent never served — {}",
            driver.events.summary()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(400));

    let events = driver.events.summary();
    assert_eq!(
        driver.events.ejects.load(Ordering::SeqCst),
        0,
        "the patched driver must never eject — {events}"
    );
    assert_eq!(
        driver.events.accepts.load(Ordering::SeqCst),
        1,
        "exactly one connection once the window opened — {events}"
    );
    let _ = agent.kill();
    let _ = agent.wait();
}
