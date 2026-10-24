//! Mock-AT-modem battery for the reader transport semantics (hardening
//! Tasks 1.2/1.3): every scenario scripts one TCP "modem" and asserts
//! both the outcome classification AND that no AT+CSIM line is ever
//! re-sent after its first full delivery (the duplicate-APDU class).

use ifd_modemifd::reader::{AtOutcome, Reader};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const SELECT: [u8; 4] = [0x00, 0xA4, 0x04, 0x00];

/// One scripted modem connection: reads AT lines, hands each to the
/// scenario closure, counts AT+CSIM deliveries globally.
struct MockModem {
    _listener_guard: Arc<()>,
    addr: String,
    csim_delivered: Arc<AtomicUsize>,
}

type Scenario = Arc<dyn Fn(&mut TcpStream, &str, usize) + Send + Sync>;

impl MockModem {
    fn spawn(scenario: Scenario) -> MockModem {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let csim = Arc::new(AtomicUsize::new(0));
        let csim_thread = Arc::clone(&csim);
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let mut conn = match conn {
                    Ok(c) => c,
                    Err(_) => break,
                };
                let csim = Arc::clone(&csim_thread);
                let scenario = Arc::clone(&scenario);
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    let mut seen = 0usize;
                    loop {
                        let mut b = [0u8; 1];
                        match conn.read(&mut b) {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {
                                if b[0] == b'\r' {
                                    let line =
                                        String::from_utf8_lossy(&buf).into_owned();
                                    if line.starts_with("AT+CSIM") {
                                        csim.fetch_add(1, Ordering::SeqCst);
                                    }
                                    seen += 1;
                                    scenario(&mut conn, &line, seen);
                                    buf.clear();
                                } else {
                                    buf.push(b[0]);
                                }
                            }
                        }
                    }
                });
            }
        });
        MockModem {
            _listener_guard: Arc::new(()),
            addr,
            csim_delivered: csim,
        }
    }

    fn delivered(&self) -> usize {
        self.csim_delivered.load(Ordering::SeqCst)
    }
}

fn echo(conn: &mut TcpStream, line: &str) {
    let _ = conn.write_all(format!("\r\n{}\r\n", line).as_bytes());
}

fn reply(conn: &mut TcpStream, extra: &str) {
    let _ = conn.write_all(format!("{}\r\n\r\nOK\r\n", extra).as_bytes());
}

fn csim_ok(conn: &mut TcpStream, rapdu_hex: &str) {
    reply(
        conn,
        &format!("+CSIM: {},\"{}\"", rapdu_hex.len(), rapdu_hex),
    )
}

fn reader_of(m: &MockModem) -> Reader {
    Reader::new(&m.addr)
}

#[test]
fn normal_exchange() {
    let m = MockModem::spawn(Arc::new(|conn, line, _| {
        echo(conn, line);
        csim_ok(conn, "9000");
    }));
    let r = reader_of(&m);
    let out = r
        .transmit_with_timeout(&SELECT, Duration::from_millis(500))
        .expect("ok");
    assert_eq!(out, vec![0x90, 0x00]);
    assert_eq!(m.delivered(), 1);
}

#[test]
fn modem_error_is_card_level_refusal() {
    let m = MockModem::spawn(Arc::new(|conn, line, _| {
        echo(conn, line);
        let _ = conn.write_all(b"\r\nERROR\r\n");
    }));
    let r = reader_of(&m);
    let out = r
        .transmit_with_timeout(&SELECT, Duration::from_millis(500))
        .expect("6F00");
    assert_eq!(out, vec![0x6F, 0x00]);
    assert_eq!(m.delivered(), 1, "ERROR refusal must not be re-sent");
}

#[test]
fn echo_then_silence_is_refused_not_retried() {
    let m = MockModem::spawn(Arc::new(|conn, line, _| {
        echo(conn, line);
        // modem heard us and goes silent (firmware-intercepted select)
    }));
    let r = reader_of(&m);
    let out = r
        .transmit_with_timeout(&SELECT, Duration::from_millis(400))
        .expect("6F00");
    assert_eq!(out, vec![0x6F, 0x00], "v3.3 semantics: post-echo silence -> 6F00");
    assert_eq!(m.delivered(), 1, "silence after echo must never be retried");
}

#[test]
fn blackhole_after_write_is_ambiguous_not_retried() {
    // The bridge swallows everything after accepting our line: no echo,
    // no answer, connection stays open. The command may already be
    // executing on the SIM — a retry would duplicate it.
    let m = MockModem::spawn(Arc::new(|_conn, _line, _| {}));
    let r = reader_of(&m);
    let out = r.transmit_with_timeout(&SELECT, Duration::from_millis(400));
    assert!(out.is_err(), "unknown delivery must surface as transport error");
    assert_eq!(m.delivered(), 1, "blackhole must never trigger a resend");
}

#[test]
fn cut_after_delivery_is_ambiguous_not_retried() {
    // socat-kill mid-session: the full line reached the modem, the
    // connection dies before any echo comes back.
    let m = MockModem::spawn(Arc::new(|conn, _line, _| {
        let _ = conn.shutdown(std::net::Shutdown::Both);
    }));
    let r = reader_of(&m);
    let out = r.transmit_with_timeout(&SELECT, Duration::from_millis(400));
    assert!(out.is_err(), "post-delivery cut must surface as transport error");
    assert_eq!(m.delivered(), 1, "post-delivery cut must never be retried");
}

#[test]
fn connection_refused_is_not_sent() {
    // nothing listens: provably not delivered; the safe retry changes
    // nothing observable, and the result is still a transport error
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = probe.local_addr().unwrap().to_string();
    drop(probe);
    let r = Reader::new(&addr);
    let out = r.transmit_with_timeout(&SELECT, Duration::from_millis(300));
    assert!(out.is_err(), "no modem -> transport error, not a fake SW");
}

#[test]
fn lost_echo_with_answer_is_distrusted() {
    // Echo lost (ATE1 off / link corruption) but a +CSIM answer + OK
    // arrives: without the echo boundary the answer is uncorrelated
    // (could be a stale response to the PREVIOUS command) and must not
    // be trusted as this exchange's card answer.
    let m = MockModem::spawn(Arc::new(|conn, _line, _| {
        csim_ok(conn, "9000");
    }));
    let r = reader_of(&m);
    let out = r.transmit_with_timeout(&SELECT, Duration::from_millis(400));
    assert!(out.is_err(), "uncorrelated +CSIM must not be trusted");
    assert_eq!(m.delivered(), 1);
}

#[test]
fn stale_csim_before_echo_is_ignored() {
    let m = MockModem::spawn(Arc::new(|conn, line, _| {
        // stale answer from a previous exchange arrives first...
        let _ = conn.write_all(b"\r\n+CSIM: 4,\"6200\"\r\n");
        // ...then the real echo + fresh answer
        echo(conn, line);
        csim_ok(conn, "9000");
    }));
    let r = reader_of(&m);
    let out = r
        .transmit_with_timeout(&SELECT, Duration::from_millis(500))
        .expect("ok");
    assert_eq!(out, vec![0x90, 0x00], "must take the post-echo answer, not the stale one");
    assert_eq!(m.delivered(), 1);
}

#[test]
fn urc_between_echo_and_answer_is_tolerated() {
    let m = MockModem::spawn(Arc::new(|conn, line, _| {
        echo(conn, line);
        let _ = conn.write_all(b"\r\n+CPIN: READY\r\n");
        csim_ok(conn, "9000");
    }));
    let r = reader_of(&m);
    let out = r
        .transmit_with_timeout(&SELECT, Duration::from_millis(500))
        .expect("ok");
    assert_eq!(out, vec![0x90, 0x00]);
}

#[test]
fn csim_length_mismatch_is_rejected() {
    let m = MockModem::spawn(Arc::new(|conn, line, _| {
        echo(conn, line);
        // declared 8 chars, actual 4: corrupt/hostile line
        reply(conn, "+CSIM: 8,\"9000\"");
    }));
    let r = reader_of(&m);
    let out = r
        .transmit_with_timeout(&SELECT, Duration::from_millis(500))
        .expect("6F00");
    assert_eq!(out, vec![0x6F, 0x00], "verified exchange + bad framing -> card-level failure");
}

#[test]
fn missing_final_ok_is_not_a_verified_answer() {
    let m = MockModem::spawn(Arc::new(|conn, line, _| {
        echo(conn, line);
        // +CSIM data but the modem never sends the final OK
        let _ = conn.write_all(b"\r\n+CSIM: 4,\"9000\"\r\n");
    }));
    let r = reader_of(&m);
    let out = r
        .transmit_with_timeout(&SELECT, Duration::from_millis(400))
        .expect("6F00");
    assert_eq!(out, vec![0x6F, 0x00], "no final result code -> refused, not Ok");
    assert_eq!(m.delivered(), 1);
}

#[test]
fn oversize_stream_drops_connection_without_resend() {
    let m = MockModem::spawn(Arc::new(|conn, line, _| {
        echo(conn, line);
        let junk = vec![b'X'; 64 * 1024];
        let _ = conn.write_all(&junk);
    }));
    let r = reader_of(&m);
    let out = r.transmit_with_timeout(&SELECT, Duration::from_millis(800));
    // echo was seen, so the bounded-drop classifies Refused -> 6F00;
    // the invariant that matters: no resend, no unbounded buffering
    assert!(matches!(out, Ok(ref sw) if sw.as_slice() == [0x6F, 0x00]) || out.is_err());
    assert_eq!(m.delivered(), 1, "flooding must not cause a resend");
}

#[test]
fn refused_target_classifies_not_sent() {
    // Deterministic NotSent: nothing to connect to. at_expect performs
    // its single safe retry and still reports NotSent (never a fake SW).
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = probe.local_addr().unwrap().to_string();
    drop(probe);
    let r = Reader::new(&addr);
    match r.at_expect("AT", "OK", Duration::from_millis(300)) {
        AtOutcome::NotSent => {}
        other => panic!("expected NotSent, got {:?}", other),
    }
}

#[test]
fn parse_csim_strictness() {
    use ifd_modemifd::at::parse_csim;
    assert_eq!(
        parse_csim("+CSIM: 4,\"9000\"\r\n\r\nOK\r\n").as_deref(),
        Some(&[0x90, 0x00][..])
    );
    assert_eq!(parse_csim("+CSIM: 0,\"\"\r\nOK").as_deref(), Some(&[][..]));
    // declared length != hex length
    assert!(parse_csim("+CSIM: 8,\"9000\"").is_none());
    // non-numeric / missing fields
    assert!(parse_csim("+CSIM: x,\"9000\"").is_none());
    assert!(parse_csim("+CSIM: 4,9000").is_none());
    assert!(parse_csim("OK").is_none()); // no +CSIM line
    // odd-length hex
    assert!(parse_csim("+CSIM: 3,\"900\"").is_none());
    // non-hex payload
    assert!(parse_csim("+CSIM: 4,\"ZZZZ\"").is_none());
    // R-APDU bound: 258 bytes = 516 chars ok, 517 declared/actual rejected
    let hex516 = "AB".repeat(258);
    assert!(parse_csim(&format!("+CSIM: 516,\"{}\"", hex516)).is_some());
    let hex518 = "AB".repeat(259);
    assert!(parse_csim(&format!("+CSIM: 518,\"{}\"", hex518)).is_none());
}

#[test]
fn parse_csim_correlates_on_echo() {
    use ifd_modemifd::at::parse_csim_in;
    let cmd = "AT+CSIM=8,\"00A4040000\"";
    // stale answer BEFORE the echo, fresh answer after: the fresh one wins
    let resp = format!(
        "\r\n+CSIM: 4,\"6200\"\r\n\r\n{}\r\n+CSIM: 4,\"9000\"\r\n\r\nOK\r\n",
        cmd
    );
    assert_eq!(
        parse_csim_in(&resp, cmd).as_deref(),
        Some(&[0x90, 0x00][..])
    );
    // only a stale answer exists after a lost echo marker -> None
    let stale_only = format!("\r\n+CSIM: 4,\"6200\"\r\n\r\nOK\r\n");
    assert!(parse_csim_in(&stale_only, cmd).is_none());
}
