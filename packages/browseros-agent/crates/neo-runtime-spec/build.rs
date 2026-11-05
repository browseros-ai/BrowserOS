//! Exports the hash of `spec/` as it is now, for the freshness check in `src/lib.rs`.
//!
//! The generated modules carry the hash of the spec they were translated from. Comparing it
//! with this one at compile time makes a spec edit without a translator run fail the build of
//! this crate and of everything that depends on it.

#[path = "codegen/spec_files.rs"]
mod spec_files;

use std::env;
use std::error::Error;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    // Naming a directory makes Cargo rerun this script when any file under it changes.
    println!("cargo::rerun-if-changed=spec");
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")?;
    let files = spec_files::read(&Path::new(&manifest_dir).join("spec"))?;
    println!(
        "cargo::rustc-env=NEO_RUNTIME_SPEC_SHA256={}",
        spec_files::sha256(&files)
    );
    Ok(())
}
