//! The two shims share an ABI; keeping this table here leaves buffer checks in one place.
use super::Backend;
use std::ffi::{c_char, c_int, c_void};

pub(super) struct Api {
    pub open: unsafe extern "C" fn(c_int, c_int, c_int, f32, c_int, *const c_char) -> *mut c_void,
    pub encode: unsafe extern "C" fn(
        *mut c_void,
        *const u8,
        *const u8,
        *const u8,
        c_int,
        c_int,
        c_int,
        c_int,
        i64,
        *mut *mut u8,
        *mut c_int,
        *mut i64,
    ) -> c_int,
    pub reconfig: unsafe extern "C" fn(*mut c_void, f32, c_int) -> c_int,
    pub close: unsafe extern "C" fn(*mut c_void),
}

impl Api {
    pub fn for_backend(backend: Backend) -> Option<Self> {
        match backend {
            #[cfg(feature = "x264")]
            Backend::X264 => Some(Self {
                open: x264::sv_open,
                encode: x264::sv_encode,
                reconfig: x264::sv_reconfig,
                close: x264::sv_close,
            }),
            #[cfg(feature = "openh264")]
            Backend::OpenH264 => Some(Self {
                open: openh264::sv_oh_open,
                encode: openh264::sv_oh_encode,
                reconfig: openh264::sv_oh_reconfig,
                close: openh264::sv_oh_close,
            }),
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }
}

#[cfg(feature = "x264")]
mod x264 {
    use super::*;
    unsafe extern "C" {
        pub fn sv_open(
            w: c_int,
            h: c_int,
            threads: c_int,
            crf: f32,
            vbv_kbps: c_int,
            preset: *const c_char,
        ) -> *mut c_void;
        pub fn sv_encode(
            e: *mut c_void,
            y: *const u8,
            u: *const u8,
            v: *const u8,
            stride_y: c_int,
            stride_uv: c_int,
            idr: c_int,
            qp: c_int,
            pts_ms: i64,
            out: *mut *mut u8,
            key: *mut c_int,
            out_pts: *mut i64,
        ) -> c_int;
        pub fn sv_reconfig(e: *mut c_void, crf: f32, vbv_kbps: c_int) -> c_int;
        pub fn sv_close(e: *mut c_void);
    }
}

#[cfg(feature = "openh264")]
mod openh264 {
    use super::*;
    unsafe extern "C" {
        pub fn sv_oh_open(
            w: c_int,
            h: c_int,
            threads: c_int,
            crf: f32,
            vbv_kbps: c_int,
            preset: *const c_char,
        ) -> *mut c_void;
        pub fn sv_oh_encode(
            e: *mut c_void,
            y: *const u8,
            u: *const u8,
            v: *const u8,
            stride_y: c_int,
            stride_uv: c_int,
            idr: c_int,
            qp: c_int,
            pts_ms: i64,
            out: *mut *mut u8,
            key: *mut c_int,
            out_pts: *mut i64,
        ) -> c_int;
        pub fn sv_oh_reconfig(e: *mut c_void, crf: f32, vbv_kbps: c_int) -> c_int;
        pub fn sv_oh_close(e: *mut c_void);
    }
}
