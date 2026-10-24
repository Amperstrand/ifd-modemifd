//! modem-vpicc — vpcd agent for a modem SIM (see agent.rs and
//! tests/hardware/VPCD-ARCHITECTURE.md). Connects OUT to a vpcd listener
//! and plays the smartcard backed by the modem; opens no listening
//! sockets. Runs router-side next to the modem (tty target) or anywhere
//! that can reach a modem bridge (tcp/ssh targets).

use ifd_modemifd::agent::{self, AgentConfig};
use ifd_modemifd::consts::FAKE_ATR;

fn main() {
    let mut vpcd_addr: Option<String> = None;
    let mut target: Option<String> = None;
    let mut atr: Vec<u8> = FAKE_ATR.to_vec();
    let mut reconnect_ms: u64 = 500;
    let mut log_path: Option<String> = None;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--vpcd" => vpcd_addr = args.next(),
            "--target" => target = args.next(),
            "--reconnect-ms" => {
                reconnect_ms = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(reconnect_ms)
            }
            "--log" => log_path = args.next(),
            "--atr-hex" => match args.next().and_then(|h| hex(&h)) {
                Some(bytes) => atr = bytes,
                None => {
                    eprintln!("modem-vpicc: --atr-hex expects even-length hex");
                    std::process::exit(2);
                }
            },
            other => {
                eprintln!("modem-vpicc: unknown arg {other}");
                usage();
                std::process::exit(2);
            }
        }
    }

    let (Some(vpcd_addr), Some(target)) = (vpcd_addr, target) else {
        usage();
        std::process::exit(2);
    };
    if reconnect_ms < 100 {
        eprintln!("modem-vpicc: --reconnect-ms floor is 100ms (bounded reconnect)");
        std::process::exit(2);
    }

    agent::run(AgentConfig {
        vpcd_addr,
        target,
        atr,
        reconnect_ms,
        log_path,
    });
}

fn usage() {
    eprintln!(
        "usage: modem-vpicc --vpcd HOST:PORT --target tcp://HOST:PORT|tty:/dev/ttyUSBx\n\
         \x20\x20\x20\x20\x20\x20\x20\x20 [--atr-hex HEX] [--reconnect-ms N] [--log PATH]"
    );
}

fn hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || s.is_empty() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}
