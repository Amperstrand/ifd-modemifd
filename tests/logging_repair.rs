//! Legacy-mode repair (Codex audit finding, PR #2): an existing log file
//! with unsafe permissions (0644, as left by the pre-v3.4.1 binary) must
//! be tightened to 0600 when the gated logger opens it — OpenOptions
//! `.mode(0o600)` only applies at creation. Own binary: the logger
//! caches its env decision and file handle in a OnceLock.

use ifd_modemifd::log;

#[test]
fn legacy_world_readable_log_is_tightened_to_0600() {
    let mut p = std::env::temp_dir();
    p.push(format!("ifd-modemifd-test-repair-{}.log", std::process::id()));
    let path = p.to_str().expect("utf8 tmp path").to_string();
    std::env::set_var("IFD_MODMIFD_LOG_PATH", &path);
    std::env::set_var("IFD_MODMIFD_DEBUG", "1");
    std::env::remove_var("IFD_MODEMIFD_LOG_APDU");
    // simulate the pre-v3.4.1 binary's world-readable log
    std::fs::write(&path, "legacy log\n").expect("seed log");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
    }
    log("line-after-upgrade");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("log exists")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "legacy 0644 log must be tightened to 0600 on open");
    }
    let content = std::fs::read_to_string(&path).expect("read");
    assert!(content.contains("line-after-upgrade"));
    let _ = std::fs::remove_file(&path);
}
