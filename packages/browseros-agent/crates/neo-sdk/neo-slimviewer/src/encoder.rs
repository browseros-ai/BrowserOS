//! One-frame-in, one-frame-out H.264 with Annex B SPS/PPS before every IDR.

use std::ffi::{CString, c_int, c_void};
use std::str::FromStr;

use crate::yuv::I420;

#[path = "encoder_native.rs"]
mod native;
#[cfg(test)]
#[path = "encoder_tests.rs"]
mod tests;

#[cfg(not(any(feature = "x264", feature = "openh264")))]
compile_error!("enable at least one encoder feature: x264 or openh264");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    X264,
    OpenH264,
}

impl Backend {
    pub fn default_name() -> &'static str {
        if cfg!(feature = "openh264") {
            "openh264"
        } else {
            "x264"
        }
    }

    fn available(self) -> bool {
        match self {
            Self::X264 => cfg!(feature = "x264"),
            Self::OpenH264 => cfg!(feature = "openh264"),
        }
    }
}

impl FromStr for Backend {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        let backend = match name {
            "x264" => Self::X264,
            "openh264" => Self::OpenH264,
            _ => return Err(format!("--encoder must be x264 or openh264, got {name:?}")),
        };
        if !backend.available() {
            return Err(format!(
                "encoder {name} is not built in; enable Cargo feature {name}"
            ));
        }
        Ok(backend)
    }
}

pub struct Settings {
    pub backend: Backend,
    pub preset: String,
    pub threads: u32,
    pub crf: f32,
}

pub struct Encoder {
    raw: *mut c_void,
    api: native::Api,
    crf: f32,
    vbv_kbps: u32,
    pub width: u16,
    pub height: u16,
}

// Both native handles may move between threads, but are never called concurrently.
unsafe impl Send for Encoder {}

pub struct Encoded {
    pub key: bool,
    pub data: Vec<u8>,
    /// The output is an earlier picture than the input (the encoder holds frames back).
    pub delayed: bool,
}

impl Encoder {
    /// Full size: x264 CRF or OpenH264 fixed QP. Half size: also target `vbv_kbps`.
    /// OpenH264's rate target is approximate: frame skipping is disabled for screen updates.
    pub fn open(
        width: u16,
        height: u16,
        settings: &Settings,
        crf: f32,
        vbv_kbps: u32,
    ) -> Option<Encoder> {
        if width == 0
            || height == 0
            || !width.is_multiple_of(2)
            || !height.is_multiple_of(2)
            || !valid_rate(crf, vbv_kbps)
            || settings.threads > 64
        {
            return None;
        }
        let api = native::Api::for_backend(settings.backend)?;
        let preset = CString::new(settings.preset.as_str()).ok()?;
        let threads = match settings.threads {
            0 => auto_threads(
                std::thread::available_parallelism().map_or(1, usize::from),
                cpu_quota(),
            ),
            n => n,
        };
        let raw = unsafe {
            (api.open)(
                c_int::from(width),
                c_int::from(height),
                threads as c_int,
                crf,
                vbv_kbps as c_int,
                preset.as_ptr(),
            )
        };
        (!raw.is_null()).then_some(Encoder {
            raw,
            api,
            crf,
            vbv_kbps,
            width,
            height,
        })
    }

    /// A sharpen request uses x264's per-frame QP, or a fine-QP IDR in OpenH264.
    pub fn encode(
        &mut self,
        pic: &I420,
        idr: bool,
        qp: Option<u8>,
        pts_ms: i64,
    ) -> Option<Encoded> {
        let pixels = usize::from(self.width) * usize::from(self.height);
        if (pic.width, pic.height) != (usize::from(self.width), usize::from(self.height))
            || pic.y.len() < pixels
            || pic.u.len() < pixels / 4
            || pic.v.len() < pixels / 4
            || qp.is_some_and(|q| q > 51)
        {
            return None;
        }
        let mut out = std::ptr::null_mut();
        let mut key = 0;
        let mut out_pts = -1;
        let size = unsafe {
            (self.api.encode)(
                self.raw,
                pic.y.as_ptr(),
                pic.u.as_ptr(),
                pic.v.as_ptr(),
                pic.width as c_int,
                (pic.width / 2) as c_int,
                c_int::from(idr),
                qp.map_or(-1, c_int::from),
                pts_ms,
                &raw mut out,
                &raw mut key,
                &raw mut out_pts,
            )
        };
        if size <= 0 || out.is_null() {
            return None;
        }
        // Each shim owns this buffer until its next encode; copy before another native call.
        let data = unsafe { std::slice::from_raw_parts(out, size as usize) }.to_vec();
        Some(Encoded {
            key: key != 0,
            data,
            delayed: out_pts != pts_ms,
        })
    }

    pub fn set_rate(&mut self, crf: f32, vbv_kbps: u32) {
        if valid_rate(crf, vbv_kbps)
            && (crf, vbv_kbps) != (self.crf, self.vbv_kbps)
            && unsafe { (self.api.reconfig)(self.raw, crf, vbv_kbps as c_int) } == 0
        {
            (self.crf, self.vbv_kbps) = (crf, vbv_kbps);
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe { (self.api.close)(self.raw) }
    }
}

fn valid_rate(crf: f32, kbps: u32) -> bool {
    crf.is_finite() && (0.0..=51.0).contains(&crf) && kbps <= (c_int::MAX / 1000) as u32
}

/// This container's CPU quota in cores (cgroup v2 `cpu.max`), if it has one.
pub fn cpu_quota() -> Option<f32> {
    let max = std::fs::read_to_string("/sys/fs/cgroup/cpu.max").ok()?;
    let mut parts = max.split_whitespace();
    let (quota, period) = (
        parts.next()?.parse::<f32>().ok()?,
        parts.next()?.parse::<f32>().ok()?,
    );
    Some(quota / period)
}

fn auto_threads(available: usize, quota: Option<f32>) -> u32 {
    let quota_limit = if quota.is_some_and(|q| q < 1.5) { 1 } else { 2 };
    (available.clamp(1, 2) as u32).min(quota_limit)
}

#[cfg(test)]
mod thread_tests {
    use super::auto_threads;
    #[test]
    fn tests_that_auto_threads_respect_available_cpus_and_quota() {
        assert_eq!(auto_threads(1, None), 1);
        assert_eq!(auto_threads(2, None), 2);
        assert_eq!(auto_threads(16, None), 2);
        assert_eq!(auto_threads(16, Some(1.0)), 1);
        assert_eq!(auto_threads(16, Some(1.5)), 2);
        assert_eq!(auto_threads(1, Some(4.0)), 1);
    }
}
