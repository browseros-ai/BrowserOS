//! `copilot-fetch.ts` body rule: when any chat message carries an `image_url` part, flag the
//! request as a vision request and shrink each `data:` image the way VS Code does (longest side
//! ≤ 2048, then shortest side ≤ 768; PNG when the image has transparency, else JPEG q75).
//! Decoding and encoding run on the blocking pool so a large screenshot never stalls the runtime.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use image::{DynamicImage, GenericImageView, ImageFormat, codecs::jpeg::JpegEncoder};
use serde_json::Value;

const MAX_LONG_SIDE: u32 = 2048;
const MAX_SHORT_SIDE: u32 = 768;

/// Returns the (possibly rewritten) body and whether it is a vision request.
pub(crate) async fn rewrite_body(body: Bytes) -> (Bytes, bool) {
    let Ok(mut json) = serde_json::from_slice::<Value>(&body) else {
        return (body, false);
    };
    if !has_image_content(&json) {
        return (body, false);
    }
    let rewritten = tokio::task::spawn_blocking(move || {
        shrink_images(&mut json);
        serde_json::to_vec(&json).ok()
    })
    .await
    .ok()
    .flatten();
    (rewritten.map(Bytes::from).unwrap_or(body), true)
}

fn image_parts_mut(body: &mut Value) -> impl Iterator<Item = &mut Value> {
    body.get_mut("messages")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .filter_map(|message| message.get_mut("content").and_then(Value::as_array_mut))
        .flatten()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("image_url"))
}

fn has_image_content(body: &Value) -> bool {
    body.get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|message| message.get("content").and_then(Value::as_array))
        .flatten()
        .any(|part| part.get("type").and_then(Value::as_str) == Some("image_url"))
}

fn shrink_images(body: &mut Value) {
    for part in image_parts_mut(body) {
        let Some(url) = part.pointer_mut("/image_url/url") else {
            continue;
        };
        let Some(data_url) = url.as_str().filter(|url| url.starts_with("data:")) else {
            continue;
        };
        match resize_data_url(data_url) {
            Ok(Some(resized)) => *url = Value::String(resized),
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "Failed to resize image for Copilot"),
        }
    }
}

/// The target size VS Code's algorithm picks, or `None` when the image is within both limits.
pub(crate) fn target_size(width: u32, height: u32) -> Option<(u32, u32)> {
    if width.max(height) <= MAX_LONG_SIDE && width.min(height) <= MAX_SHORT_SIDE {
        return None;
    }
    let (mut width, mut height) = (f64::from(width), f64::from(height));
    if width > f64::from(MAX_LONG_SIDE) || height > f64::from(MAX_LONG_SIDE) {
        let scale = f64::from(MAX_LONG_SIDE) / width.max(height);
        width = (width * scale).round();
        height = (height * scale).round();
    }
    let short = width.min(height);
    if short > f64::from(MAX_SHORT_SIDE) {
        let scale = f64::from(MAX_SHORT_SIDE) / short;
        width = (width * scale).round();
        height = (height * scale).round();
    }
    Some((width as u32, height as u32))
}

fn resize_data_url(data_url: &str) -> Result<Option<String>, String> {
    let Some((_, encoded)) = data_url.split_once(',') else {
        return Ok(None);
    };
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|error| error.to_string())?;
    let image = image::load_from_memory(&bytes).map_err(|error| error.to_string())?;
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Ok(None);
    }
    let Some((target_width, target_height)) = target_size(width, height) else {
        return Ok(None);
    };
    let resized = image.resize_exact(
        target_width,
        target_height,
        image::imageops::FilterType::Triangle,
    );
    let (mime, out) = encode(&resized)?;
    Ok(Some(format!("data:{mime};base64,{}", STANDARD.encode(out))))
}

/// Jimp's `hasAlpha()`: any pixel that is not fully opaque.
fn has_alpha(image: &DynamicImage) -> bool {
    image.color().has_alpha() && image.to_rgba8().pixels().any(|pixel| pixel.0[3] != 255)
}

fn encode(image: &DynamicImage) -> Result<(&'static str, Vec<u8>), String> {
    let mut out = Vec::new();
    if has_alpha(image) {
        image
            .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
            .map_err(|error| error.to_string())?;
        Ok(("image/png", out))
    } else {
        JpegEncoder::new_with_quality(&mut out, 75)
            .encode_image(&image.to_rgb8())
            .map_err(|error| error.to_string())?;
        Ok(("image/jpeg", out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_size_follows_vs_code() {
        assert_eq!(target_size(1000, 700), None);
        assert_eq!(target_size(2048, 768), None);
        // Longest side to 2048 first, then the short side to 768.
        assert_eq!(target_size(4096, 1024), Some((2048, 512)));
        assert_eq!(target_size(4000, 3000), Some((1024, 768)));
        assert_eq!(target_size(1920, 1080), Some((1365, 768)));
        assert_eq!(target_size(800, 800), Some((768, 768)));
    }
}
