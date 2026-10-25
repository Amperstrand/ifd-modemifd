//! Mock-AT-modem battery for the reader transport semantics (hardening
//! Tasks 1.2/1.3): every scenario scripts one TCP "modem" and asserts
//! both the outcome classification AND that no AT+CSIM line is ever
//! re-sent after its first full delivery (the duplicate-APDU class).
//! Phase-3 section (session 2) drives the exported IFDHandler C ABI
//! directly: IFDHControl GET_FEATURE_REQUEST, GetCapabilities /
//! PowerICC buffer contracts, and the LUN registry.

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
                                    let line = String::from_utf8_lossy(&buf).into_owned();
                                    if line.starts_with("AT+CSIM=") && line.contains('"') {
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
    assert_eq!(
        out,
        vec![0x6F, 0x00],
        "v3.3 semantics: post-echo silence -> 6F00"
    );
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
    assert!(
        out.is_err(),
        "unknown delivery must surface as transport error"
    );
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
    assert!(
        out.is_err(),
        "post-delivery cut must surface as transport error"
    );
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
    assert_eq!(
        out,
        vec![0x90, 0x00],
        "must take the post-echo answer, not the stale one"
    );
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
    assert_eq!(
        out,
        vec![0x6F, 0x00],
        "verified exchange + bad framing -> card-level failure"
    );
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
    assert_eq!(
        out,
        vec![0x6F, 0x00],
        "no final result code -> refused, not Ok"
    );
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

// ---------------------------------------------------------------------------
// Phase-3 IFD conformance (session 2). The reader registry is
// process-global and shared by parallel tests, so every test uses its
// own LUNs (pcscd packs reader/slot bits into the LUN — distinct values
// also prove the HashMap keys on the full c_ulong).

use ifd_modemifd::consts::{
    CM_IOCTL_GET_FEATURE_REQUEST, FAKE_ATR, IFD_COMMUNICATION_ERROR, IFD_ERROR_INSUFFICIENT_BUFFER,
    IFD_ERROR_NOT_SUPPORTED, IFD_ERROR_TAG, IFD_POWER_DOWN, IFD_POWER_UP, IFD_SUCCESS,
    RESPONSECODE, TAG_IFD_ATR, TAG_IFD_SIMULTANEOUS_ACCESS, TAG_IFD_SLOTS_NUMBER,
    TAG_IFD_THREAD_SAFE,
};
use ifd_modemifd::{
    IFDHCloseChannel, IFDHControl, IFDHCreateChannelByName, IFDHGetCapabilities, IFDHICCPresence,
    IFDHPowerICC, IFDHTransmitToICC, ScardIoHeader,
};
use std::ffi::{c_char, c_ulong};

const LUN_ATR: c_ulong = 0x5300_0001;
const LUN_POWER: c_ulong = 0x5300_0002;
const LUN_CAPS: c_ulong = 0x5300_0003;
const LUN_TX: c_ulong = 0x5300_0004;
const LUN_ISO_A: c_ulong = 0x5300_0005;
const LUN_ISO_B: c_ulong = 0x5300_0006;
const LUN_LIFECYCLE: c_ulong = 0x5300_0007;
const LUN_BAD: c_ulong = 0x5300_0008;
const LUN_ABSENT: c_ulong = 0x5300_0E0F;
const LUN_CONTROL: c_ulong = 0x5300_00F0;

fn generic_modem() -> MockModem {
    MockModem::spawn(Arc::new(|conn, line, _| {
        echo(conn, line);
        if line.starts_with("AT+CSIM=") && line.contains('"') {
            csim_ok(conn, "9000");
        } else {
            reply(conn, "");
        }
    }))
}

fn create_by_name(lun: c_ulong, addr: &str) -> RESPONSECODE {
    let name = std::ffi::CString::new(addr).unwrap();
    unsafe { IFDHCreateChannelByName(lun, name.as_ptr() as *mut c_char) }
}

fn transmit_c(lun: c_ulong, apdu: &[u8]) -> (RESPONSECODE, [u8; 64], c_ulong) {
    let mut rx = [0u8; 64];
    let mut rx_len: c_ulong = rx.len() as c_ulong;
    let rc = unsafe {
        IFDHTransmitToICC(
            lun,
            ScardIoHeader {
                protocol: 0,
                length: 0,
            },
            apdu.as_ptr(),
            apdu.len() as c_ulong,
            rx.as_mut_ptr(),
            &mut rx_len,
            std::ptr::null_mut(),
        )
    };
    (rc, rx, rx_len)
}

#[test]
fn control_get_feature_request_advertises_zero_features() {
    // pcscd probes features with TxBuffer=NULL, TxLength=0
    // (ifdwrapper.c IFDGetFeatures); zero features = success, empty.
    let mut rx = [0xA5u8; 64];
    let mut returned: c_ulong = c_ulong::MAX;
    let rc = unsafe {
        IFDHControl(
            LUN_CONTROL,
            CM_IOCTL_GET_FEATURE_REQUEST,
            std::ptr::null(),
            0,
            rx.as_mut_ptr(),
            rx.len() as c_ulong,
            &mut returned,
        )
    };
    assert_eq!(rc, IFD_SUCCESS);
    assert_eq!(returned, 0, "zero features advertised");
    assert!(
        rx.iter().all(|&b| b == 0xA5),
        "no feature TLVs may be written"
    );
}

#[test]
fn control_rejects_all_non_feature_ioctls() {
    // 0x42000001 = SCARD_CTL_CODE(1), empirically the escape channel
    // clients actually use on this pcscd/ccid bench (ACR1252 recovery
    // datapoint, 2026-10-25); 0x42000D49 = CTL_CODE(3401) (TLV
    // properties), 0x42001248 = CTL_CODE(4680) (vendor range), plus
    // arbitrary/zero codes.
    for code in [
        0u64,
        1,
        0x4200_0001,
        0x4200_0D49,
        0x4200_1248,
        0x0800_1234,
        0xDEAD_BEEF,
    ] {
        let mut rx = [0u8; 16];
        let mut returned: c_ulong = 99;
        let rc = unsafe {
            IFDHControl(
                LUN_CONTROL,
                code as c_ulong,
                std::ptr::null(),
                0,
                rx.as_mut_ptr(),
                rx.len() as c_ulong,
                &mut returned,
            )
        };
        assert_eq!(rc, IFD_ERROR_NOT_SUPPORTED, "ioctl {code:#x}");
        assert_eq!(returned, 0, "*returned zeroed on error (ifdhandler.h note)");
    }
    // a NULL returned pointer must not crash the handler
    let rc = unsafe {
        IFDHControl(
            LUN_CONTROL,
            0x1234,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, IFD_ERROR_NOT_SUPPORTED);
}

#[test]
fn power_icc_atr_buffer_contract() {
    let m = generic_modem();
    assert_eq!(create_by_name(LUN_ATR, &m.addr), IFD_SUCCESS);

    // exact size: full ATR copied, length set
    let mut atr = [0u8; FAKE_ATR.len()];
    let mut len: c_ulong = atr.len() as c_ulong;
    let rc = unsafe { IFDHPowerICC(LUN_ATR, IFD_POWER_DOWN, atr.as_mut_ptr(), &mut len) };
    assert_eq!(rc, IFD_SUCCESS);
    assert_eq!(atr, FAKE_ATR);
    assert_eq!(len as usize, FAKE_ATR.len());

    // too small: needed size reported, buffer untouched
    let mut small = [0xA5u8; 5];
    let mut len: c_ulong = 5;
    let rc = unsafe { IFDHPowerICC(LUN_ATR, IFD_POWER_DOWN, small.as_mut_ptr(), &mut len) };
    assert_eq!(rc, IFD_ERROR_INSUFFICIENT_BUFFER);
    assert_eq!(len as usize, FAKE_ATR.len());
    assert!(small.iter().all(|&b| b == 0xA5), "no partial ATR copy");

    // zero-length buffer with a real pointer: size query contract
    let mut buf = [0u8; 32];
    let mut len: c_ulong = 0;
    let rc = unsafe { IFDHPowerICC(LUN_ATR, IFD_POWER_DOWN, buf.as_mut_ptr(), &mut len) };
    assert_eq!(rc, IFD_ERROR_INSUFFICIENT_BUFFER);
    assert_eq!(len as usize, FAKE_ATR.len());

    // null pointers rejected (before any length semantics)
    let mut len: c_ulong = 32;
    let rc = unsafe { IFDHPowerICC(LUN_ATR, IFD_POWER_DOWN, std::ptr::null_mut(), &mut len) };
    assert_eq!(rc, IFD_COMMUNICATION_ERROR);
    let mut buf = [0u8; 32];
    let rc = unsafe {
        IFDHPowerICC(
            LUN_ATR,
            IFD_POWER_DOWN,
            buf.as_mut_ptr(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, IFD_COMMUNICATION_ERROR);

    // unknown LUN
    let mut len: c_ulong = 32;
    let rc = unsafe { IFDHPowerICC(LUN_ABSENT, IFD_POWER_DOWN, buf.as_mut_ptr(), &mut len) };
    assert_eq!(rc, IFD_COMMUNICATION_ERROR);
}

#[test]
fn power_icc_power_up_normalizes_session_and_returns_atr() {
    let m = generic_modem();
    assert_eq!(create_by_name(LUN_POWER, &m.addr), IFD_SUCCESS);
    assert_eq!(m.delivered(), 0, "create handshake carries no APDU CSIM");

    let mut atr = [0u8; 32];
    let mut len: c_ulong = atr.len() as c_ulong;
    let rc = unsafe { IFDHPowerICC(LUN_POWER, IFD_POWER_UP, atr.as_mut_ptr(), &mut len) };
    assert_eq!(rc, IFD_SUCCESS);
    assert_eq!(&atr[..len as usize], &FAKE_ATR[..]);
    // normalize(): close channels 1+2, select MF = 3 APDUs, each exactly once
    assert_eq!(m.delivered(), 3, "POWER_UP must run the session normalize");
}

#[test]
fn getcaps_contracts() {
    let m = generic_modem();
    assert_eq!(create_by_name(LUN_CAPS, &m.addr), IFD_SUCCESS);

    // TAG_IFD_ATR: exact size copies the ATR
    let mut val = [0u8; FAKE_ATR.len()];
    let mut len: c_ulong = val.len() as c_ulong;
    let rc = unsafe { IFDHGetCapabilities(LUN_CAPS, TAG_IFD_ATR, &mut len, val.as_mut_ptr()) };
    assert_eq!(rc, IFD_SUCCESS);
    assert_eq!(val, FAKE_ATR);
    assert_eq!(len as usize, FAKE_ATR.len());

    // too small: needed size reported, buffer untouched
    let mut small = [0xA5u8; 5];
    let mut len: c_ulong = 5;
    let rc = unsafe { IFDHGetCapabilities(LUN_CAPS, TAG_IFD_ATR, &mut len, small.as_mut_ptr()) };
    assert_eq!(rc, IFD_ERROR_INSUFFICIENT_BUFFER);
    assert_eq!(len as usize, FAKE_ATR.len());
    assert!(small.iter().all(|&b| b == 0xA5));

    // 1-byte tags: slots=1, thread-safe=1, simultaneous=16
    let mut b = [0u8; 1];
    let mut len: c_ulong = 1;
    let rc =
        unsafe { IFDHGetCapabilities(LUN_CAPS, TAG_IFD_SLOTS_NUMBER, &mut len, b.as_mut_ptr()) };
    assert_eq!(rc, IFD_SUCCESS);
    assert_eq!(b[0], 1);
    assert_eq!(len, 1);

    let mut len: c_ulong = 0;
    let rc = unsafe {
        IFDHGetCapabilities(
            LUN_CAPS,
            TAG_IFD_SLOTS_NUMBER,
            &mut len,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, IFD_ERROR_INSUFFICIENT_BUFFER, "size query via Length=0");
    assert_eq!(len, 1);

    let mut tb = [0u8; 1];
    let mut len: c_ulong = 1;
    let rc =
        unsafe { IFDHGetCapabilities(LUN_CAPS, TAG_IFD_THREAD_SAFE, &mut len, tb.as_mut_ptr()) };
    assert_eq!(rc, IFD_SUCCESS);
    assert_eq!(tb[0], 1);

    let mut sb = [0u8; 1];
    let mut len: c_ulong = 1;
    let rc = unsafe {
        IFDHGetCapabilities(
            LUN_CAPS,
            TAG_IFD_SIMULTANEOUS_ACCESS,
            &mut len,
            sb.as_mut_ptr(),
        )
    };
    assert_eq!(rc, IFD_SUCCESS);
    assert_eq!(sb[0], 16);

    // NULL Value with adequate Length must be refused, never dereferenced
    let mut len: c_ulong = 1;
    let rc = unsafe {
        IFDHGetCapabilities(
            LUN_CAPS,
            TAG_IFD_SLOTS_NUMBER,
            &mut len,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(
        rc, IFD_COMMUNICATION_ERROR,
        "NULL Value must not be written through"
    );

    // NULL Length rejected
    let rc = unsafe {
        IFDHGetCapabilities(
            LUN_CAPS,
            TAG_IFD_SLOTS_NUMBER,
            std::ptr::null_mut(),
            b.as_mut_ptr(),
        )
    };
    assert_eq!(rc, IFD_COMMUNICATION_ERROR);

    // unknown tag: IFD_ERROR_TAG + length zeroed
    let mut len: c_ulong = 8;
    let rc = unsafe { IFDHGetCapabilities(LUN_CAPS, 0x9999, &mut len, b.as_mut_ptr()) };
    assert_eq!(rc, IFD_ERROR_TAG);
    assert_eq!(len, 0);

    // unknown LUN
    let mut len: c_ulong = 32;
    let rc = unsafe { IFDHGetCapabilities(LUN_ABSENT, TAG_IFD_ATR, &mut len, val.as_mut_ptr()) };
    assert_eq!(rc, IFD_COMMUNICATION_ERROR);
}

#[test]
fn lun_registry_multi_reader_isolation() {
    let a = generic_modem();
    let b = generic_modem();
    assert_eq!(create_by_name(LUN_ISO_A, &a.addr), IFD_SUCCESS);
    assert_eq!(create_by_name(LUN_ISO_B, &b.addr), IFD_SUCCESS);

    let (rc, rx, n) = transmit_c(LUN_ISO_A, &SELECT);
    assert_eq!(rc, IFD_SUCCESS);
    assert_eq!(&rx[..n as usize], &[0x90, 0x00]);
    assert_eq!(a.delivered(), 1);
    assert_eq!(b.delivered(), 0, "reader B must not see reader A's APDU");

    let (rc, _, n) = transmit_c(LUN_ISO_B, &SELECT);
    assert_eq!(rc, IFD_SUCCESS);
    assert_eq!(n, 2);
    assert_eq!(a.delivered(), 1, "reader A untouched by B's exchange");
    assert_eq!(b.delivered(), 1);
}

#[test]
fn lun_registry_close_removes_and_recreate_replaces() {
    let a = generic_modem();
    let b = generic_modem();
    assert_eq!(create_by_name(LUN_LIFECYCLE, &a.addr), IFD_SUCCESS);
    let (rc, _, n) = transmit_c(LUN_LIFECYCLE, &SELECT);
    assert_eq!((rc, n), (IFD_SUCCESS, 2));

    assert_eq!(unsafe { IFDHCloseChannel(LUN_LIFECYCLE) }, IFD_SUCCESS);

    // every entry point must reject the removed LUN (RxLength is
    // undefined on error — only the return code is contractual)
    let (rc, _, _) = transmit_c(LUN_LIFECYCLE, &SELECT);
    assert_eq!(rc, IFD_COMMUNICATION_ERROR);
    assert_eq!(
        unsafe { IFDHICCPresence(LUN_LIFECYCLE) },
        IFD_COMMUNICATION_ERROR
    );
    let mut val = [0u8; FAKE_ATR.len()];
    let mut len: c_ulong = val.len() as c_ulong;
    let rc = unsafe { IFDHGetCapabilities(LUN_LIFECYCLE, TAG_IFD_ATR, &mut len, val.as_mut_ptr()) };
    assert_eq!(rc, IFD_COMMUNICATION_ERROR);

    // close is idempotent
    assert_eq!(unsafe { IFDHCloseChannel(LUN_LIFECYCLE) }, IFD_SUCCESS);

    // recreate the same LUN against a different modem: new target wins
    assert_eq!(create_by_name(LUN_LIFECYCLE, &b.addr), IFD_SUCCESS);
    let (rc, _, n) = transmit_c(LUN_LIFECYCLE, &SELECT);
    assert_eq!(rc, IFD_SUCCESS);
    assert_eq!(n, 2);
    assert_eq!(b.delivered(), 1, "recreated LUN routes to the new target");
    assert_eq!(a.delivered(), 1, "old target untouched after re-bind");
}

#[test]
fn create_by_name_rejects_non_transport_names() {
    assert_eq!(
        create_by_name(LUN_BAD, "not-a-socket"),
        IFD_COMMUNICATION_ERROR
    );
    let mut b = [0u8; 4];
    let mut len: c_ulong = 4;
    let rc =
        unsafe { IFDHGetCapabilities(LUN_BAD, TAG_IFD_SLOTS_NUMBER, &mut len, b.as_mut_ptr()) };
    assert_eq!(
        rc, IFD_COMMUNICATION_ERROR,
        "failed create must not register"
    );

    assert_eq!(
        unsafe { IFDHCreateChannelByName(LUN_BAD, std::ptr::null_mut()) },
        IFD_COMMUNICATION_ERROR
    );
}

#[test]
fn transmit_reports_insufficient_buffer_with_needed_size() {
    let m = generic_modem();
    assert_eq!(create_by_name(LUN_TX, &m.addr), IFD_SUCCESS);
    let mut rx = [0u8; 1];
    let mut rx_len: c_ulong = 1;
    let rc = unsafe {
        IFDHTransmitToICC(
            LUN_TX,
            ScardIoHeader {
                protocol: 0,
                length: 0,
            },
            SELECT.as_ptr(),
            SELECT.len() as c_ulong,
            rx.as_mut_ptr(),
            &mut rx_len,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, IFD_ERROR_INSUFFICIENT_BUFFER);
    assert_eq!(rx_len, 2, "needed size (SW 9000) reported");
    assert_eq!(m.delivered(), 1, "answer consumed exactly once");
}

#[test]
fn transmit_rejects_null_pointers() {
    let m = generic_modem();
    assert_eq!(create_by_name(LUN_TX, &m.addr), IFD_SUCCESS);
    let mut rx = [0u8; 4];
    let mut rx_len: c_ulong = 4;
    let rc = unsafe {
        IFDHTransmitToICC(
            LUN_TX,
            ScardIoHeader {
                protocol: 0,
                length: 0,
            },
            std::ptr::null(),
            4,
            rx.as_mut_ptr(),
            &mut rx_len,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, IFD_COMMUNICATION_ERROR);
    assert_eq!(m.delivered(), 0);
}
