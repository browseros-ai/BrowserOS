//! A returned value too large for the run's reply, made to fit by keeping its bytes with the run
//! (zuzu gap G3).
//!
//! ```text
//!  settled JSON over MAX_RESULT_BYTES ─ parse ─ every {"$neo":"bytes","base64"} in the value
//!    ─ largest first, until the value fits: replaced by {"$neo":"blob","id":"b0","size":N}
//!    ─ the run service writes each blob to outputs/<run>/blobs/<id> before it answers
//!    ─ the host reads it at GET /v1/runs/{runId}/blobs/{id}
//! ```
//!
//! Only bytes move: the bootstrap's brand for a `Buffer` or `Uint8Array`. Text never does, so a
//! value that still does not fit once every bytes value moved is `result_too_large`, and the
//! message says to return large text as `Buffer.from(text)`. A value that fits is untouched, so
//! small byte values stay inline as they always were. The settled JSON is held by the realm's
//! heap (`Defaults::JS_MEMORY_BYTES`), which bounds what this parses.

use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::io;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use neo_trace::BlobId;
use serde_json::{Value, json};

/// The brand the bootstrap gives bytes, and the one a kept blob gets in their place.
const BRAND: &str = "$neo";
const BYTES: &str = "bytes";
const BLOB: &str = "blob";

/// One byte value the reply names instead of carrying.
#[derive(Debug)]
pub(crate) struct ResultBlob {
    /// What the reply's marker names it by.
    pub(crate) id: BlobId,
    /// The bytes, decoded.
    pub(crate) bytes: Vec<u8>,
}

/// Makes `value` fit in `limit` bytes of JSON by moving its largest byte values out, and
/// returns them. Answers `Err` with the JSON length that still does not fit; `value` is then
/// partly rewritten and must not be returned.
pub(crate) fn fit_result(value: &mut Value, limit: usize) -> Result<Vec<ResultBlob>, usize> {
    let mut size = json_len(value);
    if size <= limit {
        return Ok(Vec::new());
    }
    // Each bytes value's place in a fixed walk order, with its JSON length, largest first.
    let mut found = Vec::new();
    walk(value, |place, item| {
        if let Some(base64) = bytes_brand(item) {
            found.push((place, json_len(item), base64.len()));
        }
    });
    found.sort_by_key(|&(place, length, _)| (Reverse(length), place));
    let mut moving = BTreeSet::new();
    for (place, length, base64) in found {
        if size <= limit {
            break;
        }
        moving.insert(place);
        // The marker's own length is about that of its decoded size's digits plus 40.
        size = size.saturating_sub(length) + 40 + (base64 / 4 * 3).to_string().len();
    }
    let mut blobs = Vec::new();
    walk_mut(value, |place, item| {
        if !moving.contains(&place) {
            return false;
        }
        let Some(bytes) = bytes_brand(item).and_then(|base64| STANDARD.decode(base64).ok()) else {
            return false;
        };
        let id = BlobId::new(u32::try_from(blobs.len()).unwrap_or(u32::MAX));
        *item = json!({ BRAND: BLOB, "id": id.to_string(), "size": bytes.len() });
        blobs.push(ResultBlob { id, bytes });
        true
    });
    let size = json_len(value);
    if size <= limit { Ok(blobs) } else { Err(size) }
}

/// The base64 of `value` when it is exactly the bootstrap's bytes brand.
fn bytes_brand(value: &Value) -> Option<&str> {
    let object = value.as_object()?;
    if object.len() != 2 || object.get(BRAND)?.as_str()? != BYTES {
        return None;
    }
    object.get("base64")?.as_str()
}

/// Visits every value under `root`, `root` first, each with its place in the walk. The walk is
/// iterative, so a deeply nested value cannot exhaust the thread's stack; `walk_mut` visits in
/// the same order, which is what lets a place found here name the same value there.
fn walk(root: &Value, mut visit: impl FnMut(usize, &Value)) {
    let mut stack = vec![root];
    let mut place = 0;
    while let Some(value) = stack.pop() {
        visit(place, value);
        place += 1;
        match value {
            Value::Array(items) => stack.extend(items.iter().rev()),
            Value::Object(fields) => stack.extend(fields.values().rev()),
            _ => {}
        }
    }
}

/// [`walk`], for rewriting. `visit` answers whether it replaced a bytes brand with a marker;
/// the walk then skips the brand's two children (its `$neo` and `base64` strings), which `walk`
/// counted, so the places of everything after stay the same.
fn walk_mut(root: &mut Value, mut visit: impl FnMut(usize, &mut Value) -> bool) {
    let mut stack = vec![root];
    let mut place = 0;
    while let Some(value) = stack.pop() {
        if visit(place, value) {
            place += 3;
            continue;
        }
        place += 1;
        match value {
            Value::Array(items) => stack.extend(items.iter_mut().rev()),
            Value::Object(fields) => stack.extend(fields.values_mut().rev()),
            _ => {}
        }
    }
}

/// How long `value` is as JSON, counted without building the text.
fn json_len(value: &Value) -> usize {
    let mut counter = Counter(0);
    // Writing a `Value` cannot fail, and the counter never does.
    let _counted = serde_json::to_writer(&mut counter, value);
    counter.0
}

struct Counter(usize);

impl io::Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use neo_trace::BlobId;
    use serde_json::{Value, json};

    use super::{fit_result, json_len};

    fn bytes(length: usize, fill: u8) -> Value {
        json!({ "$neo": "bytes", "base64": STANDARD.encode(vec![fill; length]) })
    }

    #[test]
    fn a_value_that_fits_is_untouched() -> Result<(), usize> {
        let mut value = json!({ "small": bytes(10, 1), "text": "hello" });
        let before = value.clone();
        assert!(fit_result(&mut value, 1024)?.is_empty());
        assert_eq!(value, before);
        Ok(())
    }

    #[test]
    fn the_largest_bytes_move_until_the_value_fits() -> Result<(), Box<dyn Error>> {
        // An array, so the walk's order is the document's whatever the map order.
        let mut value = json!([
            bytes(3000, 2),
            bytes(9000, 3),
            bytes(10, 4),
            [{ "shot": bytes(5000, 5) }],
            "kept",
        ]);
        let blobs = fit_result(&mut value, 6000).map_err(|size| format!("did not fit: {size}"))?;
        assert!(json_len(&value) <= 6000, "{value}");
        // The two largest moved, numbered in document order; the PDF and the icon stay inline.
        let moved: Vec<(BlobId, usize, u8)> = blobs
            .iter()
            .map(|blob| {
                (
                    blob.id,
                    blob.bytes.len(),
                    blob.bytes.first().copied().unwrap_or(0),
                )
            })
            .collect();
        assert_eq!(
            moved,
            vec![(BlobId::new(0), 9000, 3), (BlobId::new(1), 5000, 5)]
        );
        assert_eq!(
            value[1],
            json!({ "$neo": "blob", "id": "b0", "size": 9000 })
        );
        assert_eq!(
            value[3][0]["shot"],
            json!({ "$neo": "blob", "id": "b1", "size": 5000 })
        );
        assert_eq!(value[0], bytes(3000, 2));
        assert_eq!(value[2], bytes(10, 4));
        assert_eq!(value[4], "kept");
        Ok(())
    }

    #[test]
    fn text_never_moves() {
        let mut value = json!({ "page": "x".repeat(5000), "shot": bytes(10, 1) });
        let size = fit_result(&mut value, 1024).err();
        assert!(size.is_some_and(|size| size > 5000), "{size:?}");
    }

    #[test]
    fn a_marker_the_script_made_itself_leaves_the_walk_in_step() -> Result<(), Box<dyn Error>> {
        let mut value = json!([
            { "$neo": "blob", "id": "b9", "size": 1, "extra": [1, 2, 3] },
            bytes(4000, 8),
        ]);
        let blobs = fit_result(&mut value, 1024).map_err(|size| format!("did not fit: {size}"))?;
        assert_eq!(blobs.len(), 1);
        assert_eq!(
            value[1],
            json!({ "$neo": "blob", "id": "b0", "size": 4000 })
        );
        Ok(())
    }

    #[test]
    fn a_top_level_buffer_becomes_one_blob() -> Result<(), Box<dyn Error>> {
        let mut value = bytes(4000, 7);
        let blobs = fit_result(&mut value, 256).map_err(|size| format!("did not fit: {size}"))?;
        assert_eq!(value, json!({ "$neo": "blob", "id": "b0", "size": 4000 }));
        assert_eq!(blobs.len(), 1);
        Ok(())
    }
}
