//! Modem transport: TCP bridge (legacy) or direct SSH exec (recommended).
//!
//! The legacy path connects to a socat TCP listener on the router —
//! plaintext, unauthenticated, reachable by anything that can route to
//! the port. The SSH path opens an encrypted, host-key-pinned dropbear
//! session (key auth, BatchMode) and execs a busybox relay that owns the
//! modem AT tty directly: **no listener on the router, no socat, no
//! plaintext hop** — the same single-owner-tty doctrine as the nucula
//! CDC broker in ccid-firmware-rs.
//!
//! Target syntax (DEVICENAME in reader.conf):
//!
//! - `host:port` — legacy TCP bridge
//! - `ssh://[user@]host[:port]/dev/ttyUSBx` — SSH exec transport
//!   (user defaults to `root`, port to 22). The tty path is required:
//!   the AT port shuffles across modem resets (README modem findings),
//!   so pin the discovered port per reader.conf entry.
//!
//! Environment:
//! - `IFD_MODMIFD_SSH_KEY` — identity file for ssh
//! - `IFD_MODMIFD_SSH_CMD` — override the ssh invocation entirely
//!   (split on whitespace, prepended before the built arguments);
//!   intended for wrappers and the transport test harness.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

/// A byte pipe to the modem with the socket-style knobs the AT engine
/// needs (read deadline / nonblocking drain).
pub trait ModemLink: Read + Write + Send {
    fn set_read_timeout(&mut self, d: Option<Duration>) -> io::Result<()>;
    fn set_nonblocking(&mut self, nb: bool) -> io::Result<()>;
    /// Block until the link has readable data or `d` elapses —
    /// poll(2)-based where a raw fd exists. Returns Ok(true) when a
    /// read should be attempted (data ready OR hangup surfaced),
    /// Ok(false) on timeout. This is the AT engine's ONLY deadline
    /// primitive as of 2026-11-21 round 3: termios VMIN/VTIME proved
    /// unreliable on the real option-driver USB tty (the deployed
    /// agent hung in a blocking read with the deadline armed — pty
    /// tests could not reproduce), so boundedness must not depend on
    /// termios state at all.
    fn wait_readable(&mut self, d: Option<Duration>) -> io::Result<bool>;
}

fn poll_readable(fd: std::os::unix::io::RawFd, d: Option<Duration>) -> io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = d
        .map(|x| x.as_millis().min(i32::MAX as u128) as i32)
        .unwrap_or(-1);
    // SAFETY: poll(2) on a borrowed fd with a valid pfd; EINTR retried.
    let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
    if rc < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(err);
    }
    // POLLHUP/POLLERR also count as "readable": the following read()
    // then surfaces the EOF/error instead of the deadline spinning.
    Ok(rc > 0 && (pfd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR)) != 0)
}

impl ModemLink for TcpStream {
    fn set_read_timeout(&mut self, d: Option<Duration>) -> io::Result<()> {
        TcpStream::set_read_timeout(self, d)
    }
    fn set_nonblocking(&mut self, nb: bool) -> io::Result<()> {
        TcpStream::set_nonblocking(self, nb)
    }
    fn wait_readable(&mut self, d: Option<Duration>) -> io::Result<bool> {
        use std::os::unix::io::AsRawFd;
        poll_readable(self.as_raw_fd(), d)
    }
}

/// Open a link by target string (see module docs for the syntax).
pub fn open(target: &str) -> Option<Box<dyn ModemLink>> {
    let target = target.strip_prefix("tcp://").unwrap_or(target);
    if let Some(rest) = target.strip_prefix("tty:") {
        open_serial(rest)
    } else if let Some(rest) = target.strip_prefix("ssh://") {
        match open_ssh(rest) {
            Ok(l) => Some(Box::new(l)),
            Err(e) => {
                set_open_report(format!("ssh {target}: spawn failed: {e}"));
                crate::log(&format!("ssh transport to {} failed: {}", target, e));
                None
            }
        }
    } else {
        open_tcp(target)
    }
}

/// Human-readable trace of the LAST transport open attempt (round 3,
/// 2026-11-21: the deployed agent could only say "silent" — the
/// heartbeat now carries this so live runs show exactly how far each
/// open got: stty result, device open, deadline-arm verification).
static OPEN_REPORT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn set_open_report(msg: String) {
    *OPEN_REPORT.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg);
}

/// The last transport-open report, if any (see [`open`]).
pub fn last_open_report() -> Option<String> {
    OPEN_REPORT
    .lock()
    .unwrap_or_else(|e| e.into_inner())
    .clone()
}

/// Local tty link: termios setup via an `stty` subprocess (the crate
/// targets routers whose shells own that job; identical settings to the
/// ssh relay) and plain File I/O — read deadlines are served by the
/// AT engine's nonblocking path.
pub struct SerialLink {
    file: std::fs::File,
}

impl Read for SerialLink {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // On a tty there is no EOF: Ok(0) with VMIN=0/VTIME>0 armed can
        // only mean the read deadline expired (a genuine hangup — USB
        // modem gone — surfaces as EIO). Map it to WouldBlock so the
        // AT engine's deadline loop treats it as "no data yet" instead
        // of "peer closed" — the 2026-11-21 live hang: healthy serial
        // exchanges sit silent for 300-400 ms (card-service floor)
        // while at_full polls at 150 ms.
        match self.file.read(buf) {
            Ok(0) => Err(io::Error::new(io::ErrorKind::WouldBlock, "serial read deadline")),
            other => other,
        }
    }
}

impl Write for SerialLink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl ModemLink for SerialLink {
    /// Real read deadline via termios VMIN/VTIME — the ONLY deadline a
    /// plain tty offers (a socket-style SO_RCVTIMEO does not exist for
    /// character devices). `Some(d)` arms VMIN=0 + VTIME=ceil(d to
    /// 0.1 s) so a read can never block past ~d; `None` restores the
    /// blocking VMIN=1/VTIME=0 the open-time stty set up. Without this
    /// the AT engine's `set_read_timeout` was a no-op and a tty whose
    /// AT window died mid-exchange hung the read in the kernel forever
    /// — only the between-reads deadline check existed, which a
    /// blocked read never reaches (NR7101 live test, sha 7e979f45).
    fn set_read_timeout(&mut self, d: Option<Duration>) -> io::Result<()> {
        use std::os::unix::io::AsRawFd;
        let fd = self.file.as_raw_fd();
        let mut tio: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut tio) } != 0 {
            return Err(io::Error::last_os_error());
        }
        match d {
            Some(d) => {
                // VTIME is unsigned-char tenths of seconds (max 25.5 s)
                let tenths = (d.as_millis() + 99) / 100;
                tio.c_cc[libc::VTIME] = tenths.clamp(1, 255) as u8;
                tio.c_cc[libc::VMIN] = 0;
            }
            None => {
                tio.c_cc[libc::VTIME] = 0;
                tio.c_cc[libc::VMIN] = 1;
            }
        }
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &tio) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    fn set_nonblocking(&mut self, nb: bool) -> io::Result<()> {
        use std::os::unix::io::AsRawFd;
        let fd = self.file.as_raw_fd();
        // SAFETY: fcntl F_GETFL/F_SETFL on our own fd; flags kept as-is
        // apart from O_NONBLOCK.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        let new = if nb {
            flags | libc::O_NONBLOCK
        } else {
            flags & !libc::O_NONBLOCK
        };
        if unsafe { libc::fcntl(fd, libc::F_SETFL, new) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    fn wait_readable(&mut self, d: Option<Duration>) -> io::Result<bool> {
        use std::os::unix::io::AsRawFd;
        poll_readable(self.file.as_raw_fd(), d)
    }
}

fn open_serial(tty: &str) -> Option<Box<dyn ModemLink>> {
    let stty = Command::new("stty")
        .args(["-F", tty, "115200", "raw", "-echo", "min", "1", "time", "0"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let stty_note = match &stty {
        Ok(s) if s.success() => "stty ok".to_string(),
        Ok(s) => format!("stty exited {s} (continuing — flags may be partial)"),
        Err(e) => format!("stty spawn failed: {e}"),
    };
    match std::fs::OpenOptions::new().read(true).write(true).open(tty) {
        Ok(file) => {
            let mut link = SerialLink { file };
            // Verify the read-deadline mechanism AT OPEN: arm VMIN/VTIME
            // once and record whether termios accepted it. at_full no
            // longer depends on this (poll-gated reads), but a refusal
            // here is the live signature of a wedged USB tty.
            let arm = link
                .set_read_timeout(Some(Duration::from_millis(150)))
                .map(|_| "ok".to_string())
                .unwrap_or_else(|e| format!("REFUSED: {e}"));
            set_open_report(format!("tty {tty}: {stty_note}; open ok; termios arm {arm}"));
            Some(Box::new(link))
        }
        Err(e) => {
            set_open_report(format!("tty {tty}: {stty_note}; open FAILED: {e}"));
            crate::log(&format!("serial transport to {} failed: {}", tty, e));
            None
        }
    }
}

fn open_tcp(target: &str) -> Option<Box<dyn ModemLink>> {
    let (host, port) = target.rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;
    let mut last_err = None;
    for addr in std::net::ToSocketAddrs::to_socket_addrs(&(host, port)).ok()? {
        match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                set_open_report(format!("tcp {target}: connect ok ({addr})"));
                return Some(Box::new(s));
            }
            Err(e) => last_err = Some(e),
        }
    }
    crate::log(&format!("connect {} failed: {:?}", target, last_err));
    set_open_report(format!(
        "tcp {target}: connect failed: {}",
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "no address".into())
    ));
    None
}

/// Parsed `ssh://[user@]host[:port]/dev/tty` target.
#[derive(Debug, PartialEq, Eq)]
pub struct SshTarget {
    pub user: String,
    pub host: String,
    pub port: u16,
    pub tty: String,
}

pub fn parse_ssh_target(rest: &str) -> Option<SshTarget> {
    let (authority, tty) = rest.split_once('/')?;
    if !tty.starts_with("dev/") && !tty.starts_with("/dev/") {
        return None;
    }
    let tty = format!("/{}", tty.trim_start_matches('/'));
    let (userhost, port) = match authority.rsplit_once(':') {
        Some((uh, p)) => (uh, p.parse::<u16>().ok()?),
        None => (authority, 22),
    };
    let (user, host) = match userhost.split_once('@') {
        Some((u, h)) if !u.is_empty() && !h.is_empty() => (u.to_string(), h.to_string()),
        Some(_) => return None, // malformed: bare '@' or an empty side
        None => ("root".to_string(), userhost.to_string()),
    };
    if host.is_empty() {
        return None;
    }
    Some(SshTarget {
        user,
        host,
        port,
        tty,
    })
}

/// The remote relay, busybox-ash safe: take an exclusive flock on the
/// modem tty (multiple concurrent readers on this router byte-steal each
/// other silently — observed live as phantom transport failures), pin
/// the modem baud (a bare `raw` re-asserts the tty's stored default —
/// 9600 on some routers, which silences a 115200 modem), raw no-echo
/// with an explicit VMIN=1/VTIME=0 (this Zyxel build's stty warns
/// "unable to perform all requested operations" and leaves the tty at
/// VMIN=0/VTIME=0, where every read returns instant EOF and the relay
/// cats exit immediately), then bridge the ssh channel to the tty with
/// two cats sharing one read-write fd. The shell stays resident (no
/// exec past the cats): when the ssh side dies, the writer cat sees
/// stdin EOF, exits, and the shell kills the reader — the relay
/// self-reaps and releases the flock, so a dropped link cannot leak
/// byte-stealing relay processes onto the modem tty (the leak class
/// that corrupted an entire evening of transport verification).
const MODEM_BAUD: &str = "115200";

fn remote_relay_script(tty: &str) -> String {
    let lock = format!(
        "/var/lock/LCK..{}",
        tty.trim_start_matches('/').replace('/', "_")
    );
    format!(
        "flock -x {lock} sh -c 'stty -F {tty} {MODEM_BAUD} raw -echo min 1 time 0 2>/dev/null \
         || stty {MODEM_BAUD} raw -echo min 1 time 0 <{tty}; \
         exec 3<>{tty}; cat <&3 & R=$!; cat >&3; kill $R'"
    )
}

/// Full invocation for a target: first token is the PROGRAM, the rest
/// are its arguments (pub for tests + wrappers). Defaults to `ssh`;
/// `IFD_MODMIFD_SSH_CMD` replaces the whole prefix — e.g. a wrapper
/// (`python3 tests/fake_relay.py`) or an ssh with extra config.
pub fn ssh_command_for(t: &SshTarget) -> Vec<String> {
    let mut args: Vec<String> = std::env::var("IFD_MODMIFD_SSH_CMD")
        .unwrap_or_else(|_| "ssh".into())
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if let Ok(key) = std::env::var("IFD_MODMIFD_SSH_KEY") {
        if !key.is_empty() && !args.iter().any(|a| a == "-i") {
            args.push("-i".into());
            args.push(key);
        }
    }
    for opt in [
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-o",
        "ConnectTimeout=5",
        "-o",
        "ServerAliveInterval=10",
        "-o",
        "ServerAliveCountMax=3",
    ] {
        args.push(opt.into());
    }
    if t.port != 22 {
        args.push("-p".into());
        args.push(t.port.to_string());
    }
    args.push(format!("{}@{}", t.user, t.host));
    args.push("--".into());
    args.push(remote_relay_script(&t.tty));
    args
}

fn open_ssh(rest: &str) -> io::Result<SshLink> {
    let target = parse_ssh_target(rest)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad ssh:// target"))?;
    let mut invocation = ssh_command_for(&target).into_iter();
    let program = invocation
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty invocation"))?;
    let mut cmd = Command::new(program);
    cmd.args(invocation)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    SshLink::spawn(&mut cmd)
}

/// Encrypted link: ssh child process with piped stdio. Reads are served
/// by a pump thread (std pipes have no deadlines), so the timeout /
/// nonblocking knobs drive `recv` deadlines instead of socket options.
pub struct SshLink {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<io::Result<Vec<u8>>>,
    buf: Vec<u8>,
    buf_off: usize,
    timeout: Option<Duration>,
    nonblocking: bool,
    eof: bool,
}

impl SshLink {
    pub fn spawn(cmd: &mut Command) -> io::Result<SshLink> {
        // PDEATHSIG: if the hosting pcscd dies hard (SIGKILL — no Drop),
        // the kernel kills this ssh child too, closing the channel so
        // dropbear ends the session and the remote relay releases its
        // flock. Without it, orphaned ssh trees kept byte-stealing relay
        // processes alive on the modem tty (bench 2026-10-24, 13 reaped).
        #[cfg(unix)]
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = cmd.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "no stdout"))?;
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("modemifd-ssh-pump".into())
            .spawn(move || pump(stdout, tx))?;
        Ok(SshLink {
            child,
            stdin,
            rx,
            buf: Vec::new(),
            buf_off: 0,
            timeout: None,
            nonblocking: false,
            eof: false,
        })
    }
}

fn pump(mut stdout: ChildStdout, tx: Sender<io::Result<Vec<u8>>>) {
    let mut b = [0u8; 1024];
    loop {
        match stdout.read(&mut b) {
            Ok(0) => {
                let _ = tx.send(Ok(Vec::new()));
                return;
            }
            Ok(n) => {
                if tx.send(Ok(b[..n].to_vec())).is_err() {
                    return;
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        }
    }
}

impl Read for SshLink {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            if self.buf_off < self.buf.len() {
                let n = (self.buf.len() - self.buf_off).min(out.len());
                out[..n].copy_from_slice(&self.buf[self.buf_off..self.buf_off + n]);
                self.buf_off += n;
                return Ok(n);
            }
            if self.eof {
                return Ok(0);
            }
            let chunk = if self.nonblocking {
                match self.rx.try_recv() {
                    Ok(c) => c,
                    Err(TryRecvError::Empty) => {
                        return Err(io::Error::new(io::ErrorKind::WouldBlock, "drain"))
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.eof = true;
                        return Ok(0);
                    }
                }
            } else {
                match self.rx.recv_timeout(self.timeout.unwrap_or(Duration::MAX)) {
                    Ok(c) => c,
                    Err(RecvTimeoutError::Timeout) => {
                        return Err(io::Error::new(io::ErrorKind::WouldBlock, "timeout"))
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        self.eof = true;
                        return Ok(0);
                    }
                }
            };
            match chunk {
                Ok(v) if v.is_empty() => {
                    // remote EOF marker from the pump
                    self.eof = true;
                    return Ok(0);
                }
                Ok(v) => {
                    self.buf = v;
                    self.buf_off = 0;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Write for SshLink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stdin.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stdin.flush()
    }
}

impl ModemLink for SshLink {
    fn set_read_timeout(&mut self, d: Option<Duration>) -> io::Result<()> {
        self.timeout = d;
        Ok(())
    }
    fn set_nonblocking(&mut self, nb: bool) -> io::Result<()> {
        self.nonblocking = nb;
        Ok(())
    }
    fn wait_readable(&mut self, d: Option<Duration>) -> io::Result<bool> {
        if self.buf_off < self.buf.len() {
            return Ok(true); // buffered bytes are readable now
        }
        if self.eof {
            return Ok(true); // let read() surface the EOF
        }
        match self.rx.recv_timeout(d.unwrap_or(Duration::MAX)) {
            Ok(Ok(v)) => {
                if v.is_empty() {
                    self.eof = true; // remote EOF marker from the pump
                } else {
                    self.buf = v;
                    self.buf_off = 0;
                }
                Ok(true)
            }
            Ok(Err(e)) => Err(e),
            Err(RecvTimeoutError::Timeout) => Ok(false),
            Err(RecvTimeoutError::Disconnected) => {
                self.eof = true;
                Ok(true)
            }
        }
    }
}

impl Drop for SshLink {
    fn drop(&mut self) {
        // Flush then kill: the pipe closes with the process, and
        // dropbear SIGHUPs the remote relay when the session ends, so
        // the backgrounded `cat` on the router side does not linger.
        // stdin/stdout/rx drop naturally after this — no manual
        // ownership gymnastics needed.
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
