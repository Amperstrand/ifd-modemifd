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
}

impl ModemLink for TcpStream {
    fn set_read_timeout(&mut self, d: Option<Duration>) -> io::Result<()> {
        TcpStream::set_read_timeout(self, d)
    }
    fn set_nonblocking(&mut self, nb: bool) -> io::Result<()> {
        TcpStream::set_nonblocking(self, nb)
    }
}

/// Open a link by target string (see module docs for the syntax).
pub fn open(target: &str) -> Option<Box<dyn ModemLink>> {
    if let Some(rest) = target.strip_prefix("ssh://") {
        match open_ssh(rest) {
            Ok(l) => Some(Box::new(l)),
            Err(e) => {
                crate::log(&format!("ssh transport to {} failed: {}", target, e));
                None
            }
        }
    } else {
        open_tcp(target)
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
                return Some(Box::new(s));
            }
            Err(e) => last_err = Some(e),
        }
    }
    crate::log(&format!("connect {} failed: {:?}", target, last_err));
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

/// The remote relay, busybox-ash safe: put the tty in raw no-echo mode
/// (tty echo would corrupt the ATE1 echo-correlation; the modem's own
/// echo is the one the driver matches), then bridge the ssh channel to
/// the tty with two cats sharing one read-write fd.
fn remote_relay_script(tty: &str) -> String {
    format!(
        "stty -F {tty} raw -echo 2>/dev/null || stty raw -echo <{tty}; \
         exec 3<>{tty}; cat <&3 & exec cat >&3"
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
