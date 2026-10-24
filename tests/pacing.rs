//! Pacing-floor override — own test process: the gap is cached in a
//! OnceLock at the first paced transmit, so IFD_MODMIFD_TX_GAP_MS must
//! be set before any Reader traffic. That isolation is why this is a
//! separate test binary rather than a case in mock_at.rs.

use ifd_modemifd::reader::Reader;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SELECT: [u8; 4] = [0x00, 0xA4, 0x04, 0x00];

#[test]
fn tx_gap_env_override_is_enforced() {
    std::env::set_var("IFD_MODMIFD_TX_GAP_MS", "60");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let arrivals: Arc<Mutex<Vec<Instant>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&arrivals);
    std::thread::spawn(move || {
        if let Ok((mut conn, _)) = listener.accept() {
            let mut buf = Vec::new();
            loop {
                let mut b = [0u8; 1];
                match conn.read(&mut b) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if b[0] == b'\r' {
                            let line = String::from_utf8_lossy(&buf).into_owned();
                            buf.clear();
                            if line.starts_with("AT+CSIM=") && line.contains('"') {
                                sink.lock().unwrap().push(Instant::now());
                            }
                            let _ = conn.write_all(
                                format!("\r\n{}\r\n+CSIM: 4,\"9000\"\r\n\r\nOK\r\n", line)
                                    .as_bytes(),
                            );
                        } else {
                            buf.push(b[0]);
                        }
                    }
                }
            }
        }
    });

    let r = Reader::new(&addr);
    r.transmit_with_timeout(&SELECT, Duration::from_millis(500))
        .expect("first exchange");
    r.transmit_with_timeout(&SELECT, Duration::from_millis(500))
        .expect("second exchange");

    let t = arrivals.lock().unwrap().clone();
    assert_eq!(t.len(), 2, "both APDUs delivered exactly once");
    let gap = t[1].duration_since(t[0]).as_millis();
    assert!(
        gap >= 55,
        "override floor must apply between transmits (gap {gap} ms)"
    );
    assert!(
        gap < 390,
        "override ignored — gap {gap} ms looks like the 400 ms default"
    );
}
