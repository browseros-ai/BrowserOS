//! A stale probe declaration is a build failure, never a source edit by the test runner.
//! Request/reply examples remain the independent byte-level compatibility guard.

use std::error::Error;
use std::fs;
use std::path::Path;

#[test]
fn probe_declarations_match_the_rust_contract() -> Result<(), Box<dyn Error>> {
    let expected = neo_playwright::render_probe_protocol()?;
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../neo-playwright-js/src/probe/generated.ts");
    let committed = fs::read_to_string(&path).map_err(|error| {
        format!(
            "cannot read {}: {error}; run `bun run gen:probe` in neo-playwright-js",
            path.display()
        )
    })?;
    assert!(
        committed == expected,
        "{} is stale; run `bun run gen:probe` in neo-playwright-js, review and commit it",
        path.display()
    );
    Ok(())
}
