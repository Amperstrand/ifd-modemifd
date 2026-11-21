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
use std::sync::atomic::{AtomicUsize, Ordering};
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

    vpcd_srv.send(&[vpcd::CTRL_RESET]);
    let atr2 = vpcd_srv.read_frame().expect("ATR frame after RESET");
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
