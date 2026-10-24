//! Debug-logging security defaults (Phase 2): logging is OFF unless
//! IFD_MODMIFD_DEBUG is set. Companion on-case (incl. APDU redaction +
//! 0600 checks) lives in logging_enabled.rs — separate binaries, since
//! the logger caches its env decision in a OnceLock. Uses a private
//! IFD_MODMIFD_LOG_PATH so the bench's root-owned default log (written
//! by the still-deployed pre-hardening driver) cannot interfere.

use ifd_modemifd::log;

fn private_log_path(tag: &str) -> String {
    let mut p = std::env::temp_dir();
    p.push(format!("ifd-modemifd-test-{}-{}.log", tag, std::process::id()));
    p.to_str().expect("utf8 tmp path").to_string()
}

#[test]
fn debug_log_is_off_by_default() {
    let path = private_log_path("off");
    std::env::set_var("IFD_MODMIFD_LOG_PATH", &path);
    std::env::remove_var("IFD_MODMIFD_DEBUG");
    std::env::remove_var("IFD_MODEMIFD_LOG_APDU");
    let _ = std::fs::remove_file(&path);
    log("must-not-be-written");
    assert!(
        !std::path::Path::new(&path).exists(),
        "no debug log without IFD_MODMIFD_DEBUG"
    );
    let _ = std::fs::remove_file(&path);
}
