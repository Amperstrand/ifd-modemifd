//! ifd-modemifd — pcscd IFD handler exposing a cellular modem SIM slot as a
//! PC/SC reader. Transport: TCP bridge (socat) to the modem AT port.
//! See README for setup, caveats and the stabilization roadmap.

mod at;
mod consts;
mod reader;

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

fn log_file() -> Option<&'static Mutex<Option<std::fs::File>>> {
    static L: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();
    Some(L.get_or_init(|| {
        if true || std::env::var("IFD_MODMIFD_DEBUG").is_ok() {
            Mutex::new(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("/tmp/ifd-modemifd.log")
                    .ok(),
            )
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
        if let Some(r) = readers().lock().unwrap_or_else(|e| e.into_inner()).remove(&lun) {
            r.close();
        }
        IFD_SUCCESS
    })
}

#[no_mangle]
pub unsafe extern "C" fn IFDHTransmitToICC(
    lun: c_ulong,
    _send_pci: *mut u8, // SCARD_IO_HEADER* (opaque 8 bytes) — unused
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
        if get_reader(lun).is_none() { return IFD_COMMUNICATION_ERROR; }
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
    _code: c_ulong,
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
        IFD_SUCCESS
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
        if get_reader(lun).is_none() { return IFD_COMMUNICATION_ERROR; }
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
                *val = 1;
                *len = 1;
                IFD_SUCCESS
            }
            t if t == TAG_IFD_THREAD_SAFE as u32 || t == TAG_IFD_SIMULTANEOUS_ACCESS as u32 => {
                if (*len as usize) < 1 {
                    *len = 1;
                    return IFD_ERROR_INSUFFICIENT_BUFFER;
                }
                *val = if t == TAG_IFD_SIMULTANEOUS_ACCESS as u32 { 16 } else { 1 };
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
