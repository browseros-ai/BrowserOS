//! Neo: Binaries a `run` script returned (`RunOutcome::binaries`): `return await page.screenshot()`,
//! a nested `{shot, title}`, a PDF. Each is saved in full under `<artifacts>/returned/`, its
//! place in `value` gets a small descriptor, and a PNG or JPEG also becomes an MCP `image` block
//! the model can see, scaled down to what vision models read.
//!
//! ```text
//!  value {"shot":{"type":"Buffer","bytes":5400000}}   + binaries [{pointer:"/shot", bytes}]
//!    └─ present():  <artifacts>/returned/<stamp>-0.png   (full size, listed in artifacts[])
//!                   value.shot = {type, mimeType, bytes, width, height, file, image:{…}}
//!                   image block: ≤ 1568 px, ≤ 1.15 MP, ≤ 1 MiB (original bytes when they fit)
//! ```
//!
//! Blocking (file writes, decoding, scaling): call it off the async workers.

use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, ImageReader, Limits};
use neo_runtime::RunBinary;
use serde_json::{Value, json};

/// Neo: returned binaries share this artifact subdirectory across requests.
const RETURNED_DIR: &str = "returned";
/// Neo: longest image-block edge; the full-resolution image remains in its artifact file.
pub(super) const MAX_EDGE: u32 = 1568;
/// Neo: pixel budget per model-visible image block, independent of the JSON response budget.
pub(super) const MAX_PIXELS: u64 = 1_150_000;
/// Neo: encoded-byte cap keeps a few screenshots within the tool transport's output budget.
pub(super) const MAX_IMAGE_BYTES: usize = 1024 * 1024;
/// Neo: later images retain files and descriptors without growing the reply's image list.
pub(super) const MAX_IMAGES: usize = 8;
/// Neo: decoding at most 256 MiB of RGBA bounds blocking-worker memory. Larger files stay saved.
const MAX_DECODE_PIXELS: u64 = 64 * 1024 * 1024;
/// Neo: scaled blocks use this JPEG quality; saved original bytes retain their full quality.
const JPEG_QUALITY: u8 = 80;

/// An MCP image block's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Image {
    /// Base64, as MCP carries image data.
    pub(super) data: String,
    pub(super) mime_type: &'static str,
}

/// What a binary's bytes are, from their leading bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Png,
    Jpeg,
    Webp,
    Pdf,
    Other,
}

impl Kind {
    fn sniff(bytes: &[u8]) -> Self {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Self::Png
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            Self::Jpeg
        } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            Self::Webp
        } else if bytes.starts_with(b"%PDF-") {
            Self::Pdf
        } else {
            Self::Other
        }
    }

    fn mime_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
            Self::Pdf => "application/pdf",
            Self::Other => "application/octet-stream",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
            Self::Pdf => "pdf",
            Self::Other => "bin",
        }
    }

    fn is_image(self) -> bool {
        matches!(self, Self::Png | Self::Jpeg | Self::Webp)
    }
}

/// An image block chosen for a binary: its encoded bytes and size.
struct Shown {
    bytes: Vec<u8>,
    mime_type: &'static str,
    width: u32,
    height: u32,
}

/// Saves each binary under `dir/returned/`, replaces its placeholder in `report.value` with a
/// descriptor, lists the file in `report.artifacts`, and answers the image blocks to append to
/// the reply. A binary whose file cannot be written is still described and shown, without
/// `file`; the write error is logged.
pub(super) fn present(report: &mut Value, binaries: Vec<RunBinary>, dir: &Path) -> Vec<Image> {
    let stamp = format!(
        "{}-{:04x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_millis())
            .unwrap_or_default(),
        rand::random::<u16>()
    );
    let mut images = Vec::new();
    let mut files = Vec::new();
    for (index, binary) in binaries.into_iter().enumerate() {
        let bytes = binary.bytes;
        let kind = Kind::sniff(&bytes);
        let size = kind
            .is_image()
            .then(|| imagesize::blob_size(&bytes).ok())
            .flatten()
            .and_then(|size| {
                Some((
                    u32::try_from(size.width).ok()?,
                    u32::try_from(size.height).ok()?,
                ))
            });
        let name = format!("{stamp}-{index}.{}", kind.extension());
        let file = match save(dir, &name, &bytes) {
            Ok(file) => Some(file),
            Err(error) => {
                tracing::warn!(%error, dir = %dir.display(), "saving a returned binary failed");
                None
            }
        };
        let slot = report
            .get_mut("value")
            .and_then(|value| value.pointer_mut(&binary.pointer));
        let Some(slot) = slot else {
            continue;
        };
        let mut descriptor = json!({
            "type": slot.get("type").cloned().unwrap_or_else(|| json!("Buffer")),
            "mimeType": kind.mime_type(),
            "bytes": bytes.len(),
        });
        if let Some((width, height)) = size {
            descriptor["width"] = json!(width);
            descriptor["height"] = json!(height);
        }
        if let Some(file) = &file {
            descriptor["file"] = json!(file.display().to_string());
            files.push(json!({
                "path": format!("{RETURNED_DIR}/{name}"),
                "bytes": bytes.len(),
                "file": file.display().to_string(),
            }));
        }
        if images.len() < MAX_IMAGES
            && let Some((width, height)) = size
            && let Some(shown) = show(&bytes, kind, width, height)
        {
            descriptor["image"] = json!({
                "mimeType": shown.mime_type,
                "width": shown.width,
                "height": shown.height,
                "bytes": shown.bytes.len(),
            });
            images.push(Image {
                data: STANDARD.encode(&shown.bytes),
                mime_type: shown.mime_type,
            });
        }
        *slot = descriptor;
    }
    if !files.is_empty() {
        match report.get_mut("artifacts").and_then(Value::as_array_mut) {
            Some(listed) => listed.extend(files),
            None => report["artifacts"] = Value::Array(files),
        }
    }
    images
}

/// Writes `bytes` to `dir/returned/name`, refusing a `returned` that is a symbolic link.
fn save(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let folder = dir.join(RETURNED_DIR);
    fs::create_dir_all(&folder)?;
    if fs::symlink_metadata(&folder)?.file_type().is_symlink() {
        return Err(io::Error::other(format!(
            "{} is a symbolic link",
            folder.display()
        )));
    }
    let file = folder.join(name);
    fs::write(&file, bytes)?;
    Ok(file)
}

/// The image block for an image of `width`×`height`: its own bytes when they fit every bound,
/// otherwise a PNG or JPEG decoded, scaled to fit and encoded as JPEG (alpha flattened on
/// white). `None` when no block fits: a WebP over the bounds, an image too large to decode, or
/// bytes that do not decode.
fn show(bytes: &[u8], kind: Kind, width: u32, height: u32) -> Option<Shown> {
    let pixels = u64::from(width) * u64::from(height);
    if width.max(height) <= MAX_EDGE && pixels <= MAX_PIXELS && bytes.len() <= MAX_IMAGE_BYTES {
        return Some(Shown {
            bytes: bytes.to_vec(),
            mime_type: kind.mime_type(),
            width,
            height,
        });
    }
    let format = match kind {
        Kind::Png => ImageFormat::Png,
        Kind::Jpeg => ImageFormat::Jpeg,
        Kind::Webp | Kind::Pdf | Kind::Other => return None,
    };
    if pixels == 0 || pixels > MAX_DECODE_PIXELS {
        return None;
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_PIXELS * 4);
    reader.limits(limits);
    let decoded = match reader.decode() {
        Ok(decoded) => decoded,
        Err(error) => {
            tracing::warn!(%error, "decoding a returned image failed");
            return None;
        }
    };
    let (target_width, target_height) = fit(decoded.width(), decoded.height());
    let scaled = decoded.resize_exact(target_width, target_height, FilterType::Triangle);
    let rgb = flatten(&scaled);
    let mut encoded = Vec::new();
    if let Err(error) = JpegEncoder::new_with_quality(&mut encoded, JPEG_QUALITY).encode_image(&rgb)
    {
        tracing::warn!(%error, "encoding a returned image failed");
        return None;
    }
    (encoded.len() <= MAX_IMAGE_BYTES).then_some(Shown {
        bytes: encoded,
        mime_type: "image/jpeg",
        width: target_width,
        height: target_height,
    })
}

/// `width`×`height` scaled down, aspect kept, until the long edge is at most [`MAX_EDGE`] and
/// the area at most [`MAX_PIXELS`]; never up, never below one pixel.
#[expect(
    clippy::cast_precision_loss,
    reason = "the 1.15 megapixel cap is exactly representable and this ratio only determines image scaling"
)]
fn fit(width: u32, height: u32) -> (u32, u32) {
    let (w, h) = (f64::from(width), f64::from(height));
    let scale = (f64::from(MAX_EDGE) / w.max(h))
        .min((MAX_PIXELS as f64 / (w * h)).sqrt())
        .min(1.0);
    let side = |length: f64| {
        // In 1..=u32::MAX by construction: a positive length scaled by at most 1. The epsilon
        // keeps an exact edge (12000 × 1568/12000) from flooring to one pixel short.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "bounded positive"
        )]
        let pixels = (length * scale + 1e-6).floor().max(1.0) as u32;
        pixels
    };
    (side(w), side(h))
}

/// The image as RGB, transparent pixels blended onto white (a JPEG has no alpha).
fn flatten(image: &DynamicImage) -> DynamicImage {
    if !image.color().has_alpha() {
        return DynamicImage::ImageRgb8(image.to_rgb8());
    }
    let rgba = image.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (to, from) in rgb.pixels_mut().zip(rgba.pixels()) {
        let alpha = u16::from(from[3]);
        for channel in 0..3 {
            let blended = (u16::from(from[channel]) * alpha + 255 * (255 - alpha)) / 255;
            to[channel] = u8::try_from(blended).unwrap_or(u8::MAX);
        }
    }
    DynamicImage::ImageRgb8(rgb)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([
                u8::try_from(x % 256).unwrap(),
                u8::try_from(y % 256).unwrap(),
                128,
                255,
            ])
        });
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn tests_that_fit_keeps_aspect_within_both_bounds() {
        assert_eq!(fit(800, 600), (800, 600), "never scaled up");
        let (w, h) = fit(1032, 12000);
        assert!(
            h <= MAX_EDGE && u64::from(w) * u64::from(h) <= MAX_PIXELS,
            "{w}x{h}"
        );
        assert_eq!((w, h), (134, 1568));
        let (w, h) = fit(2560, 1440);
        assert!(
            w <= MAX_EDGE && u64::from(w) * u64::from(h) <= MAX_PIXELS,
            "{w}x{h}"
        );
        assert_eq!(fit(1, 100_000), (1, 1568), "never below one pixel");
    }

    #[test]
    fn tests_that_present_describes_saves_and_shows_nested_binaries() {
        let root = tempfile::tempdir().unwrap();
        let small = png(40, 20);
        let tall = png(600, 4000);
        let mut report = json!({"status":"ok","value":{
            "shot":{"type":"Buffer","bytes":tall.len()},
            "nested":[{"type":"Buffer","bytes":small.len()}],
            "raw":{"type":"Uint8Array","bytes":3},
            "title":"kept"
        },"artifacts":[{"path":"a.png","bytes":1,"file":"/x/a.png"}]});
        let binaries = vec![
            RunBinary {
                pointer: "/nested/0".into(),
                bytes: small.clone(),
            },
            RunBinary {
                pointer: "/raw".into(),
                bytes: vec![1, 2, 3],
            },
            RunBinary {
                pointer: "/shot".into(),
                bytes: tall.clone(),
            },
        ];
        let images = present(&mut report, binaries, root.path());
        let value = &report["value"];
        assert_eq!(value["title"], "kept");
        // A small PNG is its own block, unchanged.
        assert_eq!(value["nested"][0]["mimeType"], "image/png");
        assert_eq!(
            (
                value["nested"][0]["width"].as_u64(),
                value["nested"][0]["height"].as_u64()
            ),
            (Some(40), Some(20))
        );
        assert_eq!(
            images[0],
            Image {
                data: STANDARD.encode(&small),
                mime_type: "image/png"
            }
        );
        // Bytes that are no image: a descriptor and a file, no block.
        assert_eq!(value["raw"]["type"], "Uint8Array");
        assert_eq!(value["raw"]["mimeType"], "application/octet-stream");
        assert!(value["raw"].get("image").is_none());
        let raw_file = value["raw"]["file"].as_str().unwrap();
        assert_eq!(fs::read(raw_file).unwrap(), [1, 2, 3]);
        // A tall PNG: the full file, and a scaled JPEG block within the bounds.
        let shot = &value["shot"];
        assert_eq!(
            (shot["width"].as_u64(), shot["height"].as_u64()),
            (Some(600), Some(4000))
        );
        assert_eq!(fs::read(shot["file"].as_str().unwrap()).unwrap(), tall);
        assert_eq!(shot["image"]["mimeType"], "image/jpeg");
        assert!(shot["image"]["height"].as_u64().unwrap() <= u64::from(MAX_EDGE));
        assert_eq!(images.len(), 2);
        let block = STANDARD.decode(&images[1].data).unwrap();
        assert!(block.len() <= MAX_IMAGE_BYTES && block.starts_with(&[0xff, 0xd8, 0xff]));
        assert_eq!(images[1].mime_type, "image/jpeg");
        // Each saved file is listed after the run's own.
        let listed = report["artifacts"].as_array().unwrap();
        assert_eq!(listed.len(), 4);
        assert!(listed[1]["path"].as_str().unwrap().starts_with("returned/"));
    }

    #[test]
    fn tests_that_a_returned_binary_at_the_top_level_is_replaced_whole() {
        let root = tempfile::tempdir().unwrap();
        let small = png(10, 10);
        let mut report = json!({"status":"ok","value":{"type":"Buffer","bytes":small.len()}});
        let images = present(
            &mut report,
            vec![RunBinary {
                pointer: String::new(),
                bytes: small,
            }],
            root.path(),
        );
        assert_eq!(images.len(), 1);
        assert_eq!(report["value"]["mimeType"], "image/png");
        assert!(
            report["value"]["file"]
                .as_str()
                .unwrap()
                .ends_with("-0.png")
        );
        assert_eq!(report["artifacts"].as_array().map(Vec::len), Some(1));
    }
}
