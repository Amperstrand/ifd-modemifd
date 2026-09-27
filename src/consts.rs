//! pcsc-lite ifdhandler.h constants (values copied from
//! /usr/include/PCSC/ifdhandler.h — keep in sync).

pub type DWORD = std::ffi::c_ulong;
pub type RESPONSECODE = std::ffi::c_long;

pub const IFD_SUCCESS: RESPONSECODE = 0;
pub const IFD_ERROR_TAG: RESPONSECODE = 600;
pub const IFD_ERROR_NOT_SUPPORTED: RESPONSECODE = 606;
pub const IFD_COMMUNICATION_ERROR: RESPONSECODE = 612;
pub const IFD_NOT_SUPPORTED: RESPONSECODE = 614;
pub const IFD_ICC_PRESENT: RESPONSECODE = 615;
pub const IFD_ICC_NOT_PRESENT: RESPONSECODE = 616;
pub const IFD_ERROR_INSUFFICIENT_BUFFER: RESPONSECODE = 618;

pub const IFD_POWER_UP: DWORD = 500;
pub const IFD_POWER_DOWN: DWORD = 501;
pub const IFD_RESET: DWORD = 502;

pub const TAG_IFD_ATR: DWORD = 0x0303;
pub const TAG_IFD_THREAD_SAFE: DWORD = 0x0FAD;
pub const TAG_IFD_SLOTS_NUMBER: DWORD = 0x0FAE;
pub const TAG_IFD_SIMULTANEOUS_ACCESS: DWORD = 0x0FAF;

/// Plausible sysmo-class USIM ATR (T=0). A modem cannot report the real
/// ATR over AT commands; this is a documented limitation.
pub const FAKE_ATR: [u8; 20] = [
    0x3B, 0x9F, 0x96, 0x80, 0x1F, 0x47, 0x80, 0x31, 0xE0, 0x73, 0xF6,
    0x21, 0x1B, 0x66, 0x04, 0x20, 0x40, 0x90, 0x00, 0x73,
];
