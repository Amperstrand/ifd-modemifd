//! PC/SC sustained-load tester for a modemifd reader — the host side of the
//! pre-provisioning validation gate (tests/hardware/README.md).
//!
//! Sends N APDUs through the FULL deployed stack (pcscd -> ifd-modemifd ->
//! socat -> modem AT+CSIM -> card) and reports pass rate plus p50/p99
//! latency. Exits non-zero if any APDU fails, so it can gate a one-shot
//! eSIM provisioning attempt.
//!
//! Usage:
//!   cargo run --release --example loadtest -- [--count N] [--mode small|large|mixed]
//!
//! Modes:
//!   small — SELECT MF (7-byte command): the cheap steady-state probe.
//!   large — SELECT by DF name with a 224-byte dummy name: exercises the
//!           command sizes a profile-package (BPP) install uses. The card's
//!           expected answer is 6A82 (file not found) — counted as PASS.
//!   mixed — alternating small/large.

use std::time::Instant;

fn select_mf() -> Vec<u8> {
    vec![0x00, 0xA4, 0x00, 0x00, 0x02, 0x3F, 0x00]
}

fn select_big_dummy(n: usize) -> Vec<u8> {
    // SELECT by DF name, P2=0x0C (no response data), n filler bytes.
    // Expected SW: 6A82 (file not found) — proves the transport carried a
    // large command (BPP store-command class) and parsed the card's error.
    let mut apdu = vec![0x00, 0xA4, 0x04, 0x0C, n as u8];
    apdu.extend(std::iter::repeat(0x41).take(n));
    apdu
}

fn percentile(v: &mut [u128], p: f64) -> u128 {
    v.sort_unstable();
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[idx.min(v.len() - 1)]
}

fn main() {
    let mut count: usize = 300;
    let mut mode = "small".to_string();
    let mut size: usize = 224;
    let mut raw_apdu: Option<Vec<u8>> = None;
    let mut accept_any_sw = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--count" => count = args.next().and_then(|v| v.parse().ok()).unwrap_or(count),
            "--mode" => mode = args.next().unwrap_or_else(|| mode.clone()),
            "--size" => size = args.next().and_then(|v| v.parse().ok()).unwrap_or(size),
            "--accept-any-sw" => accept_any_sw = true,
            "--apdu" => {
                let hex = args.next().unwrap_or_default();
                let mut v = Vec::with_capacity(hex.len() / 2);
                let b = hex.as_bytes();
                let mut i = 0;
                while i + 1 < b.len() {
                    let hi = (b[i] as char).to_digit(16).unwrap_or(0) as u8;
                    let lo = (b[i + 1] as char).to_digit(16).unwrap_or(0) as u8;
                    v.push(hi << 4 | lo);
                    i += 2;
                }
                raw_apdu = Some(v);
            }
            other => {
                eprintln!("unknown arg {other}");
                std::process::exit(2);
            }
        }
    }

    let ctx = pcsc::Context::establish(pcsc::Scope::User).expect("pcscd context");
    let readers = ctx.list_readers_owned().expect("list readers");
    let reader = readers.first().unwrap_or_else(|| {
        eprintln!("no PC/SC readers — is pcscd running with the modemifd reader configured?");
        std::process::exit(2);
    });
    println!("reader: {}", reader.to_string_lossy());

    let mut card = ctx
        .connect(reader, pcsc::ShareMode::Shared, pcsc::Protocols::T0)
        .expect("SCardConnect — check polkit (access_card) if this fails as non-root");

    let mut ok = 0usize;
    let mut latencies: Vec<u128> = Vec::with_capacity(count);
    for i in 0..count {
        let apdu_owned: Vec<u8> =
            raw_apdu
                .clone()
                .unwrap_or_else(|| match (mode.as_str(), i % 2) {
                    ("large", _) => select_big_dummy(size),
                    ("mixed", 1) => select_big_dummy(size),
                    _ => select_mf(),
                });
        let apdu: &[u8] = &apdu_owned;
        let t = Instant::now();
        let mut rsp = [0u8; 512];
        let resp: &[u8] = card
            .transmit(apdu, &mut rsp)
            .expect("SCardTransmit transport");
        let dt = t.elapsed().as_millis();
        let sw = [resp[resp.len() - 2], resp[resp.len() - 1]];
        let pass = accept_any_sw || sw == [0x90, 0x00] || sw == [0x6A, 0x82];
        if pass {
            ok += 1;
            latencies.push(dt);
        } else if ok + (count - i) <= count {
            eprintln!("APDU #{i}: unexpected SW {:02X?} after {} ms", sw, dt);
        }
    }

    if latencies.is_empty() {
        println!("result: 0/{count} ok (0.0%), mode {mode} size {size}");
        std::process::exit(1);
    }
    let mut l = latencies.clone();
    let p50 = percentile(&mut l, 0.50);
    let p99 = percentile(&mut l, 0.99);
    println!(
        "result: {ok}/{count} ok ({:.1}%), p50 {p50} ms, p99 {p99} ms, mode {mode} size {size}",
        100.0 * ok as f64 / count as f64
    );
    if ok != count {
        std::process::exit(1);
    }
}
