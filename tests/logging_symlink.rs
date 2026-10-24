//! Symlink refusal (Codex audit finding, PR #2/#3): the log path must
//! never be followed through a symlink — a pre-planted link would
//! redirect an append into arbitrary files. Pins the contract that the
//! O_NOFOLLOW open enforces atomically (the pre-check alone cannot close
//! the check-to-open race). Own binary: the logger caches its env
//! decision in a OnceLock.

use ifd_modemifd::log;

#[test]
#[cfg(unix)]
fn symlinked_log_path_is_refused() {
    let mut p = std::env::temp_dir();
    p.push(format!("ifd-modemifd-test-link-{}.log", std::process::id()));
    let path = p.to_str().expect("utf8 tmp path").to_string();
    let mut c = std::env::temp_dir();
    c.push(format!(
        "ifd-modemifd-test-canary-{}.txt",
        std::process::id()
    ));
    let canary = c.to_str().expect("utf8 tmp path").to_string();
    std::fs::write(&canary, "canary\n").expect("seed canary");
    std::os::unix::fs::symlink(&canary, &path).expect("plant symlink");
    std::env::set_var("IFD_MODMIFD_LOG_PATH", &path);
    std::env::set_var("IFD_MODMIFD_DEBUG", "1");
    log("must-not-land-anywhere");
    assert_eq!(
        std::fs::read_to_string(&canary).expect("canary readable"),
        "canary\n",
        "no write may land through the symlink"
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&canary);
}
