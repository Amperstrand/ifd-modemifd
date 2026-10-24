//! Standalone scripted mock modem — the software-round counterpart of
//! tests/mock_at.rs's inline server. Echoes every AT line; APDU-carrying
//! AT+CSIM commands get `+CSIM: 4,"9000"`, everything else a bare OK.
//!
//! Usage: mockmodem [port]   (port 0 = pick free; prints `PORT <n>`)
//! Used by tests/software/pcsc_conformance.sh to back a private pcscd
//! instance loaded with the freshly built ifd-modemifd .so.

use std::io::{Read, Write};
use std::net::TcpListener;

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind 127.0.0.1");
    println!("PORT {}", listener.local_addr().expect("local addr").port());
    for conn in listener.incoming() {
        let Ok(mut conn) = conn else { continue };
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
                            let _ = conn.write_all(format!("\r\n{}\r\n", line).as_bytes());
                            if line.starts_with("AT+CSIM=") && line.contains('"') {
                                let _ = conn.write_all(b"\r\n+CSIM: 4,\"9000\"\r\n\r\nOK\r\n");
                            } else if line.starts_with("AT+CPIN?") {
                                // presence probe: a card is "in" only if
                                // +CPIN: answers — bare OK is not a match
                                let _ = conn.write_all(b"\r\n+CPIN: READY\r\n\r\nOK\r\n");
                            } else {
                                let _ = conn.write_all(b"\r\n\r\nOK\r\n");
                            }
                        } else {
                            buf.push(b[0]);
                        }
                    }
                }
            }
        });
    }
}
