//! SSH-exec transport tests: target parsing, ssh argument construction,
//! and an end-to-end exchange driven through a fake relay process
//! (tests/fake_relay.py) injected via IFD_MODMIFD_SSH_CMD — the same
//! hook a bench wrapper would use. Env-mutating tests serialize on
//! ENV_LOCK because env is process-global under the parallel test
//! runner.

use ifd_modemifd::reader::{AtOutcome, Reader};
use ifd_modemifd::transport::{parse_ssh_target, ssh_command_for};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

static ENV_LOCK_CELL: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK_CELL
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[test]
fn parse_target_variants() {
    let t = parse_ssh_target("192.168.1.1/dev/ttyUSB2").expect("bare host");
    assert_eq!(
        (t.user.as_str(), t.host.as_str(), t.port, t.tty.as_str()),
        ("root", "192.168.1.1", 22, "/dev/ttyUSB2")
    );
    let t = parse_ssh_target("bench@10.0.0.7:2222/dev/ttyUSB3").expect("full form");
    assert_eq!(
        (t.user.as_str(), t.host.as_str(), t.port),
        ("bench", "10.0.0.7", 2222)
    );
    assert_eq!(t.tty, "/dev/ttyUSB3");
    // tty must be a device path
    assert!(parse_ssh_target("host/etc/passwd").is_none());
    // empty host rejected
    assert!(parse_ssh_target("@/dev/ttyUSB2").is_none());
}

#[test]
fn ssh_command_shape() {
    let _g = env_lock();
    std::env::remove_var("IFD_MODMIFD_SSH_KEY");
    std::env::remove_var("IFD_MODMIFD_SSH_CMD");

    let t = parse_ssh_target("192.168.1.1/dev/ttyUSB2").unwrap();
    let args = ssh_command_for(&t);
    assert_eq!(
        args[0], "ssh",
        "default invocation starts with the program token"
    );
    let joined = args.join(" ");
    for must in [
        "-o BatchMode=yes",
        "-o StrictHostKeyChecking=accept-new",
        "-o ConnectTimeout=5",
        "-o ServerAliveInterval=10",
        "root@192.168.1.1",
        "--",
        "flock 9 || exit 70",
        "stty -F /dev/ttyUSB2 raw -echo",
        "exec 3<>/dev/ttyUSB2",
        "exec cat >&3",
    ] {
        assert!(joined.contains(must), "missing `{must}` in `{joined}`");
    }
    assert!(!joined.contains(" -p "), "port 22 must not add -p");

    let t = parse_ssh_target("h:2222/dev/ttyUSB1").unwrap();
    assert!(ssh_command_for(&t)
        .windows(2)
        .any(|w| w[0] == "-p" && w[1] == "2222"));

    std::env::set_var("IFD_MODMIFD_SSH_KEY", "/tmp/k");
    let args = ssh_command_for(&t);
    assert!(args.windows(2).any(|w| w[0] == "-i" && w[1] == "/tmp/k"));

    std::env::remove_var("IFD_MODMIFD_SSH_KEY");
    std::env::set_var("IFD_MODMIFD_SSH_CMD", "python3 tests/fake_relay.py");
    let args = ssh_command_for(&t);
    assert_eq!(&args[..3], &["python3", "tests/fake_relay.py", "-o"][..]);

    std::env::remove_var("IFD_MODMIFD_SSH_KEY");
    std::env::remove_var("IFD_MODMIFD_SSH_CMD");
}

#[test]
fn ssh_transport_end_to_end() {
    let _g = env_lock();
    std::env::set_var("IFD_MODMIFD_SSH_CMD", "python3 tests/fake_relay.py");
    let r = Reader::new("ssh://fake/dev/ttyUSB0");

    assert!(
        r.init_handshake(),
        "ATE1 + CSIM=? handshake through the pipe link"
    );
    assert!(r.presence(), "+CPIN: READY through the pipe link");

    let ok = r
        .transmit_with_timeout(&[0x00, 0xA4, 0x04, 0x00], Duration::from_millis(1500))
        .expect("SELECT answered");
    assert_eq!(ok, vec![0x90, 0x00]);

    // magic refusal payload: the relay answers final ERROR -> card-level 6F00
    let refused = r
        .transmit_with_timeout(&[0x42, 0x42], Duration::from_millis(1500))
        .expect("ERROR still yields an SW");
    assert_eq!(refused, vec![0x6F, 0x00]);

    r.close();
    std::env::remove_var("IFD_MODMIFD_SSH_CMD");
}

#[test]
fn ssh_transport_spawn_failure_is_not_sent() {
    let _g = env_lock();
    std::env::set_var("IFD_MODMIFD_SSH_CMD", "/nonexistent/modemifd-wrapper");
    let r = Reader::new("ssh://fake/dev/ttyUSB0");
    match r.at_expect("AT", "OK", Duration::from_millis(300)) {
        AtOutcome::NotSent => {}
        other => panic!("expected NotSent, got {:?}", other),
    }
    assert!(r
        .transmit_with_timeout(&[0x00, 0xA4, 0x04, 0x00], Duration::from_millis(300))
        .is_err());
    std::env::remove_var("IFD_MODMIFD_SSH_CMD");
}
