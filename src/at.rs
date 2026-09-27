//! APDU <-> AT+CSIM encoding helpers.

pub fn bin2hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789ABCDEF";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 0x0f) as usize] as char);
    }
    s
}

fn nib(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'F' => Some(c - b'A' + 10),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

pub fn hex2bin(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(b.len() / 2);
    for pair in b.chunks(2) {
        out.push((nib(pair[0])? << 4) | nib(pair[1])?);
    }
    Some(out)
}

/// Extract the quoted hex payload from a `+CSIM: <len>,"HEX"` line.
pub fn parse_csim(resp: &str) -> Option<Vec<u8>> {
    let line = resp.lines().find(|l| l.starts_with("+CSIM:"))?;
    let q1 = line.find('"')?;
    let rest = &line[q1 + 1..];
    let q2 = rest.find('"')?;
    hex2bin(&rest[..q2])
}

pub fn parse_cpin_present(resp: &str) -> bool {
    resp.lines().any(|l| l.starts_with("+CPIN:"))
}
