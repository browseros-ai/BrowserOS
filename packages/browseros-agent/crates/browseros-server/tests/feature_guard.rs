//! Fails when a dependency turns on a `serde_json` feature that changes `serde_json::Value` for
//! every crate in the build.
//!
//! Cargo unifies features per build, and this binary links `neo-server` to run the browser
//! engine child. `preserve_order` (an `IndexMap` behind `serde_json::Map`) or
//! `arbitrary_precision` would make the SDK, claw-server and every other member run under a
//! `serde_json` their own tests never ran, and it already broke the workspace lint once:
//! genai 0.7.5 enables `preserve_order`, which made neo-playwright and claw-server fail clippy
//! (`large_enum_variant`, `result_large_err`) in any build containing this crate. So no
//! dependency of this crate may enable either feature, on any target.

use std::process::Command;

const FORBIDDEN: [&str; 2] = ["preserve_order", "arbitrary_precision"];

#[test]
fn no_dependency_changes_serde_json_for_the_workspace() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "--manifest-path",
            manifest,
            "--package",
            "browseros-server",
            "--edges",
            "features",
            "--invert",
            "serde_json",
            "--target",
            "all",
            "--offline",
        ])
        .output()?;
    let tree = String::from_utf8(output.stdout)?;
    assert!(
        output.status.success() && tree.contains("serde_json v1"),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for feature in FORBIDDEN {
        // A feature of serde_json itself is listed at the top level of the inverted tree.
        let line = format!("── serde_json feature \"{feature}\"");
        assert!(
            !tree.lines().any(|entry| entry.contains(&line)),
            "a dependency turned on serde_json/{feature}; see the comment at the top of this file.\n{tree}"
        );
    }
    Ok(())
}
