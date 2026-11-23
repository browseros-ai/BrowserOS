//! Fails when a dependency turns on a `serde_json` feature that changes `serde_json::Value` for
//! every crate in the build.
//!
//! Cargo unifies features per build, and this binary links `neo-server` to run the browser
//! engine child. `preserve_order` (an `IndexMap` behind `serde_json::Map`) or
//! `arbitrary_precision` would make the SDK, claw-server and every other member run under a
//! `serde_json` their own tests never ran, and it already broke the workspace lint once:
//! genai 0.7.5 enables `preserve_order`, which made neo-playwright and claw-server fail clippy
//! (`large_enum_variant`, `result_large_err`) in any build containing this crate. So no
//! dependency of this crate may enable either feature. This test checks the host graph;
//! dependencies exclusive to other targets are outside its coverage. A host-only Cargo
//! cache has the data needed to run the check offline.
//!
//! It also fails when a dependency makes HTTP clients read the operating system's proxy settings.
//! reqwest's `system-proxy` (both 0.12 and 0.13 have it; 0.12's `macos-system-configuration`
//! turns it on) enables `hyper-util/client-proxy-system`, and there is one hyper-util in the
//! build, so every reqwest client in this binary would start reading the macOS and Windows proxy
//! settings, not only the client that asked: the provider client, the ChatGPT sign-in, Sentry.
//! Rig's own `rustls` feature turns it on, which is why the server takes Rig without it
//! (Cargo.toml). This check runs on the host graph, which a host-only Cargo cache always holds,
//! and on each release target whose sources this cache holds offline; a target it cannot
//! resolve is named in the test's output and not checked.

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
            "host-tuple",
            "--locked",
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

/// The release targets (`scripts/build/rust-compiler.ts`), after the host's own graph.
const TARGETS: [&str; 6] = [
    "host-tuple",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
];

/// Feature nodes of `cargo tree --edges features` that make clients read the OS proxy settings.
const OS_PROXY: [&str; 3] = [
    "hyper-util feature \"client-proxy-system\"",
    "reqwest feature \"system-proxy\"",
    "reqwest feature \"macos-system-configuration\"",
];

#[test]
#[allow(
    clippy::print_stderr,
    reason = "names each target this Cargo cache cannot resolve offline"
)]
fn no_dependency_makes_http_clients_read_os_proxy_settings()
-> Result<(), Box<dyn std::error::Error>> {
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    for target in TARGETS {
        let output = Command::new(env!("CARGO"))
            .args([
                "tree",
                "--manifest-path",
                manifest,
                "--package",
                "browseros-server",
                "--edges",
                "features",
                "--prefix",
                "none",
                "--target",
                target,
                "--locked",
                "--offline",
            ])
            .output()?;
        if !output.status.success() {
            assert!(
                target != "host-tuple",
                "cargo tree failed for the host: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            eprintln!("not checked: {target} cannot be resolved offline from this Cargo cache");
            continue;
        }
        let tree = String::from_utf8(output.stdout)?;
        assert!(
            tree.contains("hyper-util v"),
            "the {target} graph has no hyper-util, so this check would pass vacuously"
        );
        for feature in OS_PROXY {
            assert!(
                !tree.lines().any(|line| line.contains(feature)),
                "on {target}, a dependency turned on {feature}: every reqwest client in the binary \
                 would read the OS proxy settings; see the comment at the top of this file"
            );
        }
    }
    Ok(())
}
