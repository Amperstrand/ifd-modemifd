//! Enabled-logging companion to logging.rs (own binary: the logger
//! caches its env decision in a OnceLock, so the on-case cannot share a
//! process with the off-case). Uses a private IFD_MODMIFD_LOG_PATH so
//! the bench's root-owned default log cannot interfere.

use ifd_modemifd::reader::Reader;
use std::time::Duration;

#[test]
fn apdu_hex_not_logged_by_default() {
    let mut p = std::env::temp_dir();
    p.push(format!("ifd-modemifd-test-on-{}.log", std::process::id()));
    let path = p.to_str().expect("utf8 tmp path").to_string();
    std::env::set_var("IFD_MODMIFD_LOG_PATH", &path);
    std::env::set_var("IFD_MODMIFD_DEBUG", "1");
    std::env::remove_var("IFD_MODEMIFD_LOG_APDU");
    let _ = std::fs::remove_file(&path);
    // modem-less transmit: connect fails, but the XMIT log line is
    // written before the exchange starts
    let r = Reader::new("127.0.0.1:1");
    let _ = r.transmit_with_timeout(&[0x00, 0xA4, 0x04, 0x00], Duration::from_millis(100));
    let content = std::fs::read_to_string(&path).expect("log exists when enabled");
    assert!(
        !content.contains("00A40400"),
        "APDU hex must not reach the log without IFD_MODEMIFD_LOG_APDU"
    );
    assert!(content.contains("XMIT"), "length-only XMIT line expected");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("log exists")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "log must be 0600");
    }
    let _ = std::fs::remove_file(&path);
}
