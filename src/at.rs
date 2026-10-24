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

/// Max R-APDU the parser accepts: 256 data bytes + SW (TS 27.007 CSIM
/// carries a response APDU; >516 hex chars is a malformed/hostile line).
const CSIM_MAX_RESPONSE_HEX: usize = 516;

/// Extract the quoted hex payload from a `+CSIM: <len>,"HEX"` line,
/// strictly: the declared length MUST equal the hex-character count
/// (TS 27.007 §8.45 — same convention as +CGLA/+CRSM; pySim's modem
/// transport encodes `AT+CSIM=%d,"%s"` with the hex-string length) and
/// the payload MUST fit an R-APDU. Returns None on any mismatch.
pub fn parse_csim(resp: &str) -> Option<Vec<u8>> {
    parse_csim_lines(resp.lines())
}

/// Echo-correlated parse: only `+CSIM:` lines AFTER the last occurrence
/// of `echo` (our command echo) answer this exchange — a stale `+CSIM`
/// from a lagging previous exchange arriving before the echo must never
/// be mistaken for this command's card answer (response-lag aliasing,
/// conwrt-bench #22).
pub fn parse_csim_in(resp: &str, echo: &str) -> Option<Vec<u8>> {
    let at = resp.rfind(echo)?;
    parse_csim_lines(resp[at + echo.len()..].lines())
}

fn parse_csim_lines<'a>(mut lines: impl Iterator<Item = &'a str>) -> Option<Vec<u8>> {
    let line = lines.find(|l| l.starts_with("+CSIM:"))?;
    let rest = line.strip_prefix("+CSIM:")?.trim_start();
    let (lenstr, quoted) = rest.split_once(',')?;
    let declared: usize = lenstr.trim().parse().ok()?;
    let q1 = quoted.find('"')?;
    let hexpart = &quoted[q1 + 1..];
    let q2 = hexpart.find('"')?;
    let hexs = &hexpart[..q2];
    if declared != hexs.len() || hexs.len() > CSIM_MAX_RESPONSE_HEX {
        return None;
    }
    hex2bin(hexs)
}

pub fn parse_cpin_present(resp: &str) -> bool {
    resp.lines().any(|l| l.starts_with("+CPIN:"))
}
