//! A capture's coverage, shown the same way by every tool that observes a page.
//!
//! Core decides what a capture read; this only presents it. The warning holds BrowserOS's own
//! report and no page text, so it sits outside the untrusted-content fence, and it never joins
//! the snapshot text, so it cannot enter a diff baseline or read as a page change.

use browseros_core::snapshot::{FrameError, SnapshotCoverage};
use serde_json::{Map, Value};

/// `partial`, and `frameErrors` when a frame failed, for a tool's structured result. Present
/// only where a capture was taken: absent coverage is never reported as complete.
fn coverage_fields(coverage: &SnapshotCoverage, uncompared: &[FrameError]) -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert("partial".to_string(), Value::Bool(coverage.is_partial()));
    if coverage.is_partial() {
        fields.insert(
            "frameErrors".to_string(),
            errors_json(&coverage.frame_errors),
        );
    }
    if !uncompared.is_empty() {
        fields.insert("uncomparedFrames".to_string(), errors_json(uncompared));
    }
    fields
}

pub fn insert_coverage(target: &mut Value, coverage: &SnapshotCoverage, uncompared: &[FrameError]) {
    if let Value::Object(object) = target {
        object.extend(coverage_fields(coverage, uncompared));
    }
}

fn errors_json(errors: &[FrameError]) -> Value {
    serde_json::to_value(errors).unwrap_or(Value::Null)
}

/// Lines saying which frames a capture did not fully read and which a diff left out; `None`
/// for a complete capture compared in full.
pub fn coverage_warning(coverage: &SnapshotCoverage, uncompared: &[FrameError]) -> Option<String> {
    let mut lines = Vec::new();
    if coverage.is_partial() {
        let count = coverage.frame_errors.len();
        lines.push(format!(
            "Partial capture: {count} {} not fully read, so content {} hold is unknown, not absent.",
            plural(count, "frame was", "frames were"),
            plural(count, "it would", "they would"),
        ));
        lines.extend(coverage.frame_errors.iter().map(|error| {
            let effect = if error.contents_missing() {
                "contents not read"
            } else {
                "clickable elements not scanned"
            };
            format!(
                "- {}: {effect} ({}: {})",
                frame_label(error),
                wire_name(&error.stage),
                wire_name(&error.reason)
            )
        }));
    }
    if !uncompared.is_empty() {
        let count = uncompared.len();
        lines.push(format!(
            "Not compared: {count} {} read differently by the previous capture, so {} left out of this diff; take a snapshot to see {}.",
            plural(count, "frame was", "frames were"),
            plural(count, "its lines are", "their lines are"),
            plural(count, "it", "them"),
        ));
        lines.extend(
            uncompared
                .iter()
                .map(|error| format!("- {}", frame_label(error))),
        );
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// `warning` above `text`, so it stays visible however the text is cut down.
pub fn with_warning(warning: Option<&str>, text: String) -> String {
    match warning {
        Some(warning) => format!("{warning}\n{text}"),
        None => text,
    }
}

fn frame_label(error: &FrameError) -> String {
    match (&error.frame_id, &error.owner) {
        (Some(frame_id), _) => format!("frame {frame_id}"),
        (None, Some(owner)) => format!(
            "iframe element {} in {}",
            owner.backend_node_id,
            owner.parent.as_ref().map_or_else(
                || "the main frame".to_string(),
                |parent| format!("frame {parent}")
            )
        ),
        (None, None) => "main frame".to_string(),
    }
}

fn wire_name(value: &impl serde::Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(name)) => name,
        _ => String::new(),
    }
}

fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 { one } else { many }
}
