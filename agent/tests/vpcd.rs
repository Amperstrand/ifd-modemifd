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
    for (byte, name) in [(0x00u8, "OFF"), (0x02, "ATR"), (0x04, "RESET")] {
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
