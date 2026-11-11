//! Background native tabs cannot reliably paint a single very tall surface. Capture bounded
//! strips without changing the viewport or foreground tab, then stream their rows into one
//! lossless PNG. A bounded channel keeps PNG compression off the async protocol executor.

use std::io::{Cursor, Write};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use browseros_core::screenshot::ScreenshotCapture;
use png::{BitDepth, ColorType, Decoder, Encoder, Reader};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use tokio::task;

use super::screenshot_geometry::ScreenshotGeometry;
use crate::op_scope::OpScope;
use crate::{ErrorCode, PlaywrightError};

const MAX_STRIP_HEIGHT: u32 = 16_000;
const MAX_STRIP_PIXELS: f64 = 8_000_000.0;

/// One encoded strip and its expected image height. Native captures must not silently crop.
struct Tile {
    data: String,
    height: u32,
    encoded_height: u32,
}

pub(super) fn needs_tiles(geometry: &ScreenshotGeometry) -> bool {
    geometry.clip.height * geometry.raster_scale > f64::from(MAX_STRIP_HEIGHT)
}

pub(super) async fn capture(
    capture: &ScreenshotCapture,
    mut params: Map<String, Value>,
    geometry: &ScreenshotGeometry,
    scope: &OpScope<'_>,
) -> Result<String, PlaywrightError> {
    let height = pixels(geometry.clip.height.floor() * geometry.raster_scale).ok_or_else(|| {
        scope.fail(
            ErrorCode::InvalidInput,
            "Screenshot dimensions are too large",
        )
    })?;
    // At most one encoded strip waits behind the decoder. The joiner retains only PNG row
    // buffers and the compressed result, never width * total height * 4 bytes of RGBA pixels.
    let (sender, receiver) = mpsc::channel(1);
    let joined = task::spawn_blocking(move || join(height, receiver));
    let sent = produce(capture, &mut params, geometry, height, &sender, scope).await;
    drop(sender);
    let result = joined
        .await
        .map_err(|error| scope.fail(ErrorCode::Internal, &error.to_string()))?;
    sent?;
    result.map_err(|error| scope.fail(ErrorCode::Internal, &error))
}

async fn produce(
    capture: &ScreenshotCapture,
    params: &mut Map<String, Value>,
    geometry: &ScreenshotGeometry,
    height: u32,
    sender: &mpsc::Sender<Tile>,
    scope: &OpScope<'_>,
) -> Result<(), PlaywrightError> {
    let scale = geometry.raster_scale;
    let max_css_height = (MAX_STRIP_PIXELS / (geometry.clip.width * scale * scale))
        .min(f64::from(MAX_STRIP_HEIGHT) / scale)
        .floor()
        .max(1.0);
    let mut top = 0;
    while top < height {
        // Chromium truncates clip dimensions to CSS integers before scaling its output.
        // Origins remain fractional: advance by actual raster rows to keep the pixel phase.
        let css_height = (f64::from(height - top) / scale).ceil().min(max_css_height);
        let encoded_height = pixels(css_height * scale)
            .ok_or_else(|| scope.fail(ErrorCode::InvalidInput, "Invalid PNG strip dimensions"))?;
        let rows = encoded_height.min(height - top);
        let clip = geometry.clip;
        params.insert(
            "clip".to_owned(),
            json!({
                "x": clip.x, "y": clip.y + f64::from(top) / scale,
                "width": clip.width, "height": css_height, "scale": clip.scale,
            }),
        );
        let data = capture
            .capture(Value::Object(params.clone()))
            .await
            .map_err(|error| scope.core_error(&error))?;
        if sender
            .send(Tile {
                data,
                height: rows,
                encoded_height,
            })
            .await
            .is_err()
        {
            // Only the joiner can close this channel; its result carries the codec's cause.
            break;
        }
        top += rows;
    }
    Ok(())
}

// CDP uses positive raster dimensions. The limit also avoids lossy float-to-integer casts.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "range checked before conversion"
)]
fn pixels(value: f64) -> Option<u32> {
    let value = value.round();
    (value.is_finite() && value >= 1.0 && value <= f64::from(u32::MAX)).then_some(value as u32)
}

type TileReader = Reader<Cursor<Vec<u8>>>;

fn decode(tile: &Tile) -> Result<TileReader, String> {
    let bytes = STANDARD
        .decode(&tile.data)
        .map_err(|error| error.to_string())?;
    let reader = Decoder::new(Cursor::new(bytes))
        .read_info()
        .map_err(|error| error.to_string())?;
    let info = reader.info();
    if info.height != tile.encoded_height
        || info.interlaced
        || info.bit_depth != BitDepth::Eight
        || !matches!(info.color_type, ColorType::Rgb | ColorType::Rgba)
    {
        return Err("Browser returned an unexpected PNG strip format or height".to_owned());
    }
    Ok(reader)
}

fn join(height: u32, mut receiver: mpsc::Receiver<Tile>) -> Result<String, String> {
    let mut tile = receiver
        .blocking_recv()
        .ok_or("Screenshot ended before its first strip")?;
    let mut reader = decode(&tile)?;
    // Keep Chromium's color profile and density. RGB strips become RGBA so opaque and
    // transparent sections can share one PNG; no full-frame pixel buffer is allocated.
    let mut info = reader.info().clone();
    let width = info.width;
    info.height = height;
    info.color_type = ColorType::Rgba;
    info.sbit = None;
    info.bkgd = None;
    let mut output = Vec::new();
    let mut writer = Encoder::with_info(&mut output, info)
        .and_then(Encoder::write_header)
        .map_err(|error| error.to_string())?;
    {
        let mut stream = writer.stream_writer().map_err(|error| error.to_string())?;
        let mut rows = 0_u32;
        loop {
            rows = rows
                .checked_add(tile.height)
                .filter(|rows| *rows <= height)
                .ok_or("Browser returned too many PNG rows")?;
            copy_rows(&mut reader, &mut stream, width, tile.height)?;
            let Some(next) = receiver.blocking_recv() else {
                break;
            };
            tile = next;
            reader = decode(&tile)?;
        }
        if rows != height {
            return Err("Screenshot ended before all PNG strips arrived".to_owned());
        }
        stream.finish().map_err(|error| error.to_string())?;
    }
    writer.finish().map_err(|error| error.to_string())?;
    Ok(STANDARD.encode(output))
}

fn copy_rows(
    reader: &mut TileReader,
    stream: &mut impl Write,
    width: u32,
    rows: u32,
) -> Result<(), String> {
    if reader.info().width != width {
        return Err("Browser changed PNG strip width".to_owned());
    }
    let rgb = reader.info().color_type == ColorType::Rgb;
    let mut rgba = Vec::new();
    for _ in 0..rows {
        let row = reader
            .next_row()
            .map_err(|error| error.to_string())?
            .ok_or("Browser returned too few PNG strip rows")?;
        let data = if rgb {
            rgba.clear();
            for pixel in row.data().as_chunks::<3>().0 {
                rgba.extend_from_slice(pixel);
                rgba.push(255);
            }
            &rgba
        } else {
            row.data()
        };
        stream.write_all(data).map_err(|error| error.to_string())?;
    }
    Ok(())
}
