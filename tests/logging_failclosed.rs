//! Fail-closed permission repair (Codex audit round 2, PR #12): when the
//! 0600 tightening fails (fchmod EPERM on a writable file owned by
//! another user), the logger must refuse to write — never append APDU
//! payloads to a file it could not secure. Reproduces Codex's
//! nobody-vs-root-owned-0666 scenario when passwordless sudo is
//! available; skips with a note otherwise. Own binary: OnceLock-cached
//! logger env/handle.

use ifd_modemifd::log;

/// Inner probe: appends one line through the gated logger. Only acts
/// when invoked via the outer test (LOGPROBE_AS_NOBODY set); a direct
/// `cargo test` run makes it a no-op so it can never pollute the outer
/// assertion.
#[test]
fn probe_append() {
    if std::env::var("LOGPROBE_AS_NOBODY").is_err() {
        return;
    }
    log("PROBE-SECRET");
}

#[test]
fn unsecurable_log_fails_closed() {
    if std::env::var("LOGPROBE_AS_NOBODY").is_ok() {
        return; // inner invocation
    }
    let sudo_ok = matches!(
        std::process::Command::new("sudo")
            .args(["-n", "-u", "nobody", "true"])
            .output(),
        Ok(o) if o.status.success()
    );
    if !sudo_ok {
        eprintln!("SKIP: no passwordless sudo — cannot stage the EPERM scenario");
        return;
    }
    let mut p = std::env::temp_dir();
    p.push(format!("ifd-modemifd-test-fc-{}", std::process::id()));
    std::fs::create_dir_all(&p).expect("dir");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // non-sticky world-writable dir: needed so the kernel's
        // fs.protected_regular=2 (sticky-dir O_CREAT owner check) does
        // not refuse the open before the fchmod-EPERM class is reached
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o777));
    }
    p.push("log");
    let path = p.to_str().expect("utf8 tmp path").to_string();
    // world-writable file owned by us: nobody can open+append but
    // cannot fchmod it — exactly the EPERM repair-failure class
    std::fs::write(&path, "seed\n").expect("seed");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666));
    }
    // test binaries live under a home dir nobody cannot traverse; run a
    // copy from /tmp
    let mut probe_bin = std::env::temp_dir();
    probe_bin.push(format!("ifd-modemifd-logprobe-{}", std::process::id()));
    let probe_bin = probe_bin.to_str().expect("utf8 tmp path").to_string();
    std::fs::copy(std::env::current_exe().expect("exe"), &probe_bin).expect("copy probe");
    let out = std::process::Command::new("sudo")
        .args(["-n", "-u", "nobody", "env"])
        .arg("LOGPROBE_AS_NOBODY=1")
        .arg("IFD_MODMIFD_DEBUG=1")
        .arg(format!("IFD_MODMIFD_LOG_PATH={}", path))
        .arg(&probe_bin)
        .args(["--exact", "probe_append", "--nocapture"])
        .output()
        .expect("run probe");
    assert!(out.status.success(), "probe run failed: {:?}", out);
    let content = std::fs::read_to_string(&path).expect("read");
    assert!(
        !content.contains("PROBE-SECRET"),
        "unsecurable log must fail closed: nothing may be appended to a file the logger could not secure"
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&probe_bin);
    let _ = std::fs::remove_dir(p.parent().expect("parent"));
}
