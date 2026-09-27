#[path = "../src/consts.rs"] mod consts;
#[path = "../src/at.rs"] mod at;
#[path = "../src/reader.rs"] mod reader;
use reader::Reader;
fn main() {
    let r = Reader::new("192.168.13.124:7002");
    println!("handshake: {}", r.init_handshake());
    match r.at("AT+CPIN?", std::time::Duration::from_millis(2500)) {
        reader::AtResult::Ok(s) => println!("CPIN resp: {:?}", s),
        other => println!("CPIN other: {:?}", matches!(other, reader::AtResult::Err(_))),
    }
    for apdu in [
        vec![0x00,0xA4,0x00,0x0C,0x02,0x3F,0x00],
        vec![0x00,0xA4,0x08,0x0C,0x02,0x2F,0xE2],
        vec![0x00,0xB0,0x00,0x00,0x0A],
    ] {
        match r.transmit(&apdu) {
            Ok(v) => println!("APDU {:02X?} -> {} bytes: {:02X?}", apdu, v.len(), &v[..v.len().min(20)]),
            Err(_) => println!("APDU {:02X?} -> TRANSPORT ERR", apdu),
        }
    }
}
