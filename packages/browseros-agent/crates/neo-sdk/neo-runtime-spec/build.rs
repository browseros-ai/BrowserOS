//! Exports the hash of `spec/` as it is now, for the freshness check in `src/lib.rs`, and the
//! switch that builds the translator without that check.
//!
//! The generated modules carry the hash of the spec they were translated from. Comparing it
//! with this one at compile time makes a spec edit without a translator run fail the build of
//! this crate and of everything that depends on it.
//!
//! Cargo links a package's library into its examples, so the translator (`codegen/`, an
//! example) could not build while that check fails, which is exactly when it has to run.
//! `NEO_RUNTIME_SPEC_CODEGEN` (any value) sets `cfg(neo_runtime_spec_codegen)`, which builds the
//! library empty, so neither a stale hash nor a broken generated file can stop the translator
//! that replaces them.

#[path = "codegen/spec_files.rs"]
mod spec_files;

use std::env;
use std::error::Error;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    // Naming a directory makes Cargo rerun this script when any file under it changes.
    println!("cargo::rerun-if-changed=spec");
    println!("cargo::rerun-if-env-changed=NEO_RUNTIME_SPEC_CODEGEN");
    println!("cargo::rustc-check-cfg=cfg(neo_runtime_spec_codegen)");
    if env::var_os("NEO_RUNTIME_SPEC_CODEGEN").is_some() {
        println!("cargo::rustc-cfg=neo_runtime_spec_codegen");
    }
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")?;
    let files = spec_files::read(&Path::new(&manifest_dir).join("spec"))?;
    println!(
        "cargo::rustc-env=NEO_RUNTIME_SPEC_SHA256={}",
        spec_files::sha256(&files)
    );
    Ok(())
}
