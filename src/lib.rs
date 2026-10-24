//! ifd-modemifd — pcscd IFD handler exposing a cellular modem SIM slot as a
//! PC/SC reader. Transport: TCP bridge (socat) to the modem AT port.
//! See README for setup, caveats and the stabilization roadmap.

pub mod at;
pub mod consts;
pub mod reader;

use consts::*;
use reader::Reader;
use std::collections::HashMap;
use std::ffi::{c_char, c_ulong, CStr};
use std::fs::OpenOptions;
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, OnceLock};

type Readers = Mutex<HashMap<c_ulong, Arc<Reader>>>;

fn readers() -> &'static Readers {
    static R: OnceLock<Readers> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Debug log: opt-in via IFD_MODMIFD_DEBUG, created 0600, never through
/// a symlink (fixed /tmp path — a pre-planted link would redirect an
/// append into arbitrary files). APDU CONTENT is additionally gated
/// behind IFD_MODEMIFD_LOG_APDU (see reader.rs): authentication and
/// provisioning payloads must not land on disk by default.
fn debug_log_path() -> String {
    std::env::var("IFD_MODMIFD_LOG_PATH").unwrap_or_else(|_| "/tmp/ifd-modemifd.log".to_string())
}

fn open_debug_log() -> Option<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = debug_log_path();
    if std::fs::symlink_metadata(&path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return None;
    }
    // O_NOFOLLOW closes the check-to-open race (a link planted between
    // the pre-check above and the open) on arches sharing the generic
    // Linux flag value; elsewhere the pre-check remains the guard.
    #[cfg(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    ))]
    let mut open_opts = {
        const O_NOFOLLOW: i32 = 0o400000;
        let mut o = std::fs::OpenOptions::new();
        o.custom_flags(O_NOFOLLOW);
        o
    };
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    let mut open_opts = std::fs::OpenOptions::new();
    let f = open_opts
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)
        .ok()?;
    // `.mode()` only applies at creation: tighten files left looser by
    // earlier binaries (the pre-v3.4.1 0644 log) via the fd — fchmod,
    // race-free, never touching a different inode than we opened. If
    // the tightening fails (EPERM on a writable file owned by another
    // user), fail CLOSED: no log handle, no append — secrets never land
    // in a file the logger could not secure.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if f.set_permissions(std::fs::Permissions::from_mode(0o600))
            .is_err()
        {
            return None;
        }
    }
    Some(f)
}

fn log_file() -> Option<&'static Mutex<Option<std::fs::File>>> {
    static L: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();
    Some(L.get_or_init(|| {
        if std::env::var("IFD_MODMIFD_DEBUG").is_ok() {
            Mutex::new(open_debug_log())
        } else {
            Mutex::new(None)
        }
    }))
}

pub fn log(msg: &str) {
    if let Some(m) = log_file() {
        if let Ok(mut g) = m.lock() {
            if let Some(f) = g.as_mut() {
                let _ = writeln!(f, "{}", msg);
            }
        }
    }
}

fn guard<F: FnOnce() -> RESPONSECODE>(f: F) -> RESPONSECODE {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(IFD_COMMUNICATION_ERROR)
}

unsafe fn cstr<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    CStr::from_ptr(p).to_str().ok()
}

fn get_reader(lun: c_ulong) -> Option<Arc<Reader>> {
    readers()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&lun)
        .cloned()
}

fn open_reader(lun: c_ulong, target: &str) -> RESPONSECODE {
    let r = Arc::new(Reader::new(target));
    if !r.init_handshake() {
        log(&format!("open {} FAILED handshake", target));
        // keep it registered anyway: presence probe will retry connecting
    }
    readers()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(lun, r);
    log(&format!("CreateChannel lun={:#x} target={}", lun, target));
    IFD_SUCCESS
}

#[no_mangle]
pub unsafe extern "C" fn IFDHCreateChannel(lun: c_ulong, _channel: c_ulong) -> RESPONSECODE {
    guard(|| {
        let target = std::env::var("IFD_MODMIFD_TARGET").unwrap_or_default();
        if target.is_empty() {
            return IFD_COMMUNICATION_ERROR;
        }
        open_reader(lun, &target)
    })
}

#[no_mangle]
pub unsafe extern "C" fn IFDHCreateChannelByName(
    lun: c_ulong,
    device_name: *mut c_char,
) -> RESPONSECODE {
    guard(|| match cstr(device_name) {
        Some(t) if t.contains(':') => open_reader(lun, t),
        _ => IFD_COMMUNICATION_ERROR,
    })
}

#[no_mangle]
pub unsafe extern "C" fn IFDHCloseChannel(lun: c_ulong) -> RESPONSECODE {
    guard(|| {
        if let Some(r) = readers()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&lun)
        {
            r.close();
        }
        IFD_SUCCESS
    })
}

/// pcsc-lite passes SCARD_IO_HEADER BY VALUE as the 2nd parameter of
/// IFDHTransmitToICC. DWORD = unsigned long = 8 bytes on LP64, so this is a
/// 16-byte struct occupying TWO integer registers — it must be declared as
/// a by-value repr(C) struct or every following argument lands in the wrong
/// register (the bug behind the zero-blob responses: conwrt-bench #22).
#[repr(C)]
pub struct ScardIoHeader {
    pub protocol: c_ulong,
    pub length: c_ulong,
}

#[no_mangle]
pub unsafe extern "C" fn IFDHTransmitToICC(
    lun: c_ulong,
    _send_pci: ScardIoHeader,
    tx: *const u8,
    tx_len: c_ulong,
    rx: *mut u8,
    rx_len: *mut c_ulong,
    _recv_pci: *mut u8,
) -> RESPONSECODE {
    guard(|| {
        let r = match get_reader(lun) {
            Some(r) => r,
            None => return IFD_COMMUNICATION_ERROR,
        };
        if tx.is_null() || rx.is_null() || rx_len.is_null() || tx_len == 0 {
            return IFD_COMMUNICATION_ERROR;
        }
        let apdu = std::slice::from_raw_parts(tx, tx_len as usize);
        match r.transmit(apdu) {
            Ok(data) => {
                let cap = *rx_len as usize;
                if data.len() > cap {
                    *rx_len = data.len() as c_ulong;
                    return IFD_ERROR_INSUFFICIENT_BUFFER;
                }
                std::ptr::copy_nonoverlapping(data.as_ptr(), rx, data.len());
                *rx_len = data.len() as c_ulong;
                IFD_SUCCESS
            }
            Err(()) => {
                *rx_len = 0;
                IFD_COMMUNICATION_ERROR
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn IFDHPowerICC(
    lun: c_ulong,
    _action: c_ulong,
    atr: *mut u8,
    atr_len: *mut c_ulong,
) -> RESPONSECODE {
    guard(|| {
        let r = match get_reader(lun) {
            Some(r) => r,
            None => return IFD_COMMUNICATION_ERROR,
        };
        if _action == IFD_POWER_UP {
            r.normalize();
        }
        if atr.is_null() || atr_len.is_null() {
            return IFD_COMMUNICATION_ERROR;
        }
        if (*atr_len as usize) < FAKE_ATR.len() {
            *atr_len = FAKE_ATR.len() as c_ulong;
            return IFD_ERROR_INSUFFICIENT_BUFFER;
        }
        std::ptr::copy_nonoverlapping(FAKE_ATR.as_ptr(), atr, FAKE_ATR.len());
        *atr_len = FAKE_ATR.len() as c_ulong;
        IFD_SUCCESS
    })
}

#[no_mangle]
pub unsafe extern "C" fn IFDHSetProtocolParameters(
    _lun: c_ulong,
    _proto: c_ulong,
    _f: u8,
    _p1: u8,
    _p2: u8,
    _p3: u8,
) -> RESPONSECODE {
    IFD_SUCCESS
}

#[no_mangle]
pub unsafe extern "C" fn IFDHSetCapabilities(
    _lun: c_ulong,
    _tag: c_ulong,
    _len: c_ulong,
    _val: *mut u8,
) -> RESPONSECODE {
    IFD_SUCCESS
}

#[no_mangle]
pub unsafe extern "C" fn IFDHControl(
    _lun: c_ulong,
    code: c_ulong,
    _tx: *const u8,
    _txl: c_ulong,
    _rx: *mut u8,
    _rxl: c_ulong,
    returned: *mut c_ulong,
) -> RESPONSECODE {
    guard(|| {
        if !returned.is_null() {
            *returned = 0;
        }
        if code == CM_IOCTL_GET_FEATURE_REQUEST {
            // This driver advertises ZERO features: no TLV is written,
            // zero bytes returned, IFD_SUCCESS (pcscd then registers an
            // empty feature list instead of erroring the reader).
            IFD_SUCCESS
        } else {
            IFD_ERROR_NOT_SUPPORTED
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn IFDHICCPresence(lun: c_ulong) -> RESPONSECODE {
    guard(|| match get_reader(lun) {
        Some(r) if r.presence() => IFD_ICC_PRESENT,
        Some(_) => IFD_ICC_NOT_PRESENT,
        None => IFD_COMMUNICATION_ERROR,
    })
}

#[no_mangle]
pub unsafe extern "C" fn IFDHGetCapabilities(
    lun: c_ulong,
    tag: c_ulong,
    len: *mut c_ulong,
    val: *mut u8,
) -> RESPONSECODE {
    guard(|| {
        if get_reader(lun).is_none() {
            return IFD_COMMUNICATION_ERROR;
        }
        if len.is_null() {
            return IFD_COMMUNICATION_ERROR;
        }
        match tag as u32 {
            t if t == TAG_IFD_ATR as u32 => {
                if (*len as usize) < FAKE_ATR.len() {
                    *len = FAKE_ATR.len() as c_ulong;
                    return IFD_ERROR_INSUFFICIENT_BUFFER;
                }
                if val.is_null() {
                    return IFD_COMMUNICATION_ERROR;
                }
                std::ptr::copy_nonoverlapping(FAKE_ATR.as_ptr(), val, FAKE_ATR.len());
                *len = FAKE_ATR.len() as c_ulong;
                IFD_SUCCESS
            }
            t if t == TAG_IFD_SLOTS_NUMBER as u32 => {
                if (*len as usize) < 1 {
                    *len = 1;
                    return IFD_ERROR_INSUFFICIENT_BUFFER;
                }
                if val.is_null() {
                    return IFD_COMMUNICATION_ERROR;
                }
                *val = 1;
                *len = 1;
                IFD_SUCCESS
            }
            t if t == TAG_IFD_THREAD_SAFE as u32 || t == TAG_IFD_SIMULTANEOUS_ACCESS as u32 => {
                if (*len as usize) < 1 {
                    *len = 1;
                    return IFD_ERROR_INSUFFICIENT_BUFFER;
                }
                if val.is_null() {
                    return IFD_COMMUNICATION_ERROR;
                }
                *val = if t == TAG_IFD_SIMULTANEOUS_ACCESS as u32 {
                    16
                } else {
                    1
                };
                *len = 1;
                IFD_SUCCESS
            }
            _ => {
                *len = 0;
                IFD_ERROR_TAG
            }
        }
    })
}
