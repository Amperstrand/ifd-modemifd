//! pcscd-level conformance battery for ifd-modemifd (issue #7: by-value
//! SCARD_IO_HEADER ABI smoke; software pre-round for issue #8: GET_FEATURE,
//! ATR, warm reset, escape rejection, multi-reader, TX_GAP env). Runs
//! against a pcscd instance configured by tests/software/
//! pcsc_conformance.sh with mock modems — not standalone.
//!
//! Prints one PASS/FAIL line per contract check; exit 1 on any FAIL.

use ifd_modemifd::consts::FAKE_ATR;
use std::time::Instant;

/// pcsc-lite reader.h: CM_IOCTL_GET_FEATURE_REQUEST = SCARD_CTL_CODE(3400)
const GET_FEATURE: u64 = 0x4200_0000 + 3400;
/// vendor-escape range sample (SCARD_CTL_CODE(4680)) — must be refused
const VENDOR_ESCAPE: u64 = 0x4200_0000 + 4680;

const SELECT_MF: [u8; 7] = [0x00, 0xA4, 0x00, 0x0C, 0x02, 0x3F, 0x00];

fn main() {
    let mut failures = 0usize;
    let mut check = |name: &str, ok: bool, detail: String| {
        println!("{} {name} — {detail}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            failures += 1;
        }
    };

    let ctx = pcsc::Context::establish(pcsc::Scope::User).expect("SCardEstablishContext");
    let readers = ctx.list_readers_owned().expect("SCardListReaders");
    let names: Vec<String> = readers
        .iter()
        .map(|r| r.to_string_lossy().into_owned())
        .collect();
    println!("readers: {names:?}");

    let find = |tag: &str| {
        readers
            .iter()
            .find(|r| r.to_string_lossy().contains(tag))
            .cloned()
    };
    let reader_a = find("MIFD_A");
    let reader_b = find("MIFD_B");
    check(
        "readers_listed",
        reader_a.is_some() && reader_b.is_some(),
        format!(
            "MIFD_A={} MIFD_B={}",
            reader_a.is_some(),
            reader_b.is_some()
        ),
    );
    let reader_a = match reader_a {
        Some(r) => r,
        None => {
            eprintln!("MIFD_A reader absent — aborting remaining checks");
            std::process::exit(1);
        }
    };

    // SCardConnect drives IFDHPowerICC(POWER_UP) → session normalize.
    let card = ctx.connect(&reader_a, pcsc::ShareMode::Shared, pcsc::Protocols::T0);
    let mut card = match card {
        Ok(c) => {
            check("connect_a", true, "shared/T0".to_string());
            c
        }
        Err(e) => {
            check("connect_a", false, format!("SCardConnect: {e:?}"));
            eprintln!("no card session — aborting remaining checks");
            println!("RESULT RED 1");
            std::process::exit(1);
        }
    };

    // The v3.2 evidence gap: SCARD_IO_HEADER crosses the IFDHandler ABI
    // BY VALUE (16 bytes, two registers on LP64). A wrong declaration
    // lands every later argument in the wrong register — the zero-blob
    // response class (conwrt-bench #22). A correct SW here is the proof.
    let t = Instant::now();
    let mut rsp = [0u8; 512];
    let resp = card
        .transmit(&SELECT_MF, &mut rsp)
        .expect("SCardTransmit SELECT MF");
    let sw = [resp[resp.len() - 2], resp[resp.len() - 1]];
    check(
        "transmit_abi_select_mf",
        sw == [0x90, 0x00],
        format!("SW {:02X?} in {} ms", sw, t.elapsed().as_millis()),
    );

    let st = card.status2_owned().expect("SCardStatus MIFD_A");
    check(
        "atr_is_fake_atr",
        st.atr() == FAKE_ATR,
        format!(
            "{} bytes, matches sysmo-class fake ATR: {}",
            st.atr().len(),
            st.atr() == FAKE_ATR
        ),
    );

    let reconnect = card.reconnect(
        pcsc::ShareMode::Shared,
        pcsc::Protocols::T0,
        pcsc::Disposition::ResetCard,
    );
    let atr2_ok =
        reconnect.is_ok() && matches!(&card.status2_owned(), Ok(s2) if s2.atr() == FAKE_ATR);
    check(
        "reconnect_reset_atr_stable",
        atr2_ok,
        format!("reconnect={:?}", reconnect.is_ok()),
    );

    let mut ctlbuf = [0u8; 64];
    let feature = card.control(GET_FEATURE, &[], &mut ctlbuf);
    check(
        "get_feature_zero_features",
        matches!(&feature, Ok(v) if v.is_empty()),
        match &feature {
            Ok(v) => format!("Ok, {} bytes", v.len()),
            Err(e) => format!("Err {e:?}"),
        },
    );

    let escape = card.control(VENDOR_ESCAPE, &[0x00], &mut ctlbuf);
    check(
        "vendor_escape_rejected",
        escape.is_err(),
        match &escape {
            Ok(v) => format!("WRONG: Ok({} bytes)", v.len()),
            Err(e) => format!("Err {e:?}"),
        },
    );

    // Pacing floor via pcscd's env (IFD_MODMIFD_TX_GAP_MS=60 in the
    // runner): a paced transmit must not return before the floor.
    let t = Instant::now();
    let _resp = card
        .transmit(&SELECT_MF, &mut rsp)
        .expect("paced SCardTransmit");
    let paced_ms = t.elapsed().as_millis();
    check(
        "tx_gap_env_floor",
        paced_ms >= 50 && paced_ms <= 2000,
        format!("{paced_ms} ms (floor 60 expected, 400 = env ignored)"),
    );

    // Second reader through the same pcscd: independent session; the
    // first card keeps answering afterwards (registry isolation).
    if let Some(rb) = reader_b {
        let mut card_b = ctx.connect(&rb, pcsc::ShareMode::Shared, pcsc::Protocols::T0);
        let b_ok = match &mut card_b {
            Ok(cb) => {
                matches!(cb.transmit(&SELECT_MF, &mut rsp), Ok(r) if r.ends_with(&[0x90, 0x00]))
            }
            Err(e) => {
                println!("connect MIFD_B failed: {e:?}");
                false
            }
        };
        let resp = card.transmit(&SELECT_MF, &mut rsp);
        let a_again = matches!(resp, Ok(r) if r.ends_with(&[0x90, 0x00]));
        check(
            "multi_reader_sessions",
            b_ok && a_again,
            format!("B={} A-after={}", b_ok, a_again),
        );
        if let Ok(cb) = card_b {
            let _ = cb.disconnect(pcsc::Disposition::LeaveCard);
        }
    }

    let disc = card.disconnect(pcsc::Disposition::LeaveCard);
    check("disconnect", disc.is_ok(), "leave card".to_string());

    println!(
        "RESULT {} {}",
        if failures == 0 { "GREEN" } else { "RED" },
        failures
    );
    if failures != 0 {
        std::process::exit(1);
    }
}
