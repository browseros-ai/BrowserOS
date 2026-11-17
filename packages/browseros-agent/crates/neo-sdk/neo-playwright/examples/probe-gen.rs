//! Explicit writer for the probe contract. Tests call the same pure renderer and compare;
//! they never regenerate files while a builder is working in the checkout.

use std::error::Error;
use std::path::Path;
use std::{fs, io};

fn main() -> Result<(), Box<dyn Error>> {
    use std::io::Write as _;

    let output = neo_playwright::render_probe_protocol()?;
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../neo-playwright-js/src/probe/generated.ts");
    fs::write(&path, output)?;
    writeln!(io::stdout().lock(), "Wrote {}", path.display())?;
    Ok(())
}
