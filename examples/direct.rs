use ifd_modemifd::reader::{AtOutcome, Reader};
fn main() {
    let target = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: direct <ip:port>  (socat modem-AT bridge)");
        std::process::exit(2)
    });
    let r = Reader::new(&target);
    println!("handshake: {}", r.init_handshake());
    match r.at_expect("AT+CPIN?", "+CPIN:", std::time::Duration::from_millis(2500)) {
        AtOutcome::Ok(s) => println!("CPIN resp: {:?}", s),
        other => println!("CPIN other: {:?}", other),
    }
    for apdu in [
        vec![0x00, 0xA4, 0x00, 0x0C, 0x02, 0x3F, 0x00],
        vec![0x00, 0xA4, 0x08, 0x0C, 0x02, 0x2F, 0xE2],
        vec![0x00, 0xB0, 0x00, 0x00, 0x0A],
    ] {
        match r.transmit(&apdu) {
            Ok(v) => println!(
                "APDU {:02X?} -> {} bytes: {:02X?}",
                apdu,
                v.len(),
                &v[..v.len().min(20)]
            ),
            Err(_) => println!("APDU {:02X?} -> TRANSPORT ERR", apdu),
        }
    }
}
