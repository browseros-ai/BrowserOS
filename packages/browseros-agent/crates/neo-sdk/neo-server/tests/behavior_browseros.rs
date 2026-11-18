//! Neo-specific policy, tab ownership and nested-frame regressions, separate from upstream
//! conformance. The Node rig uses the real built binary and the same four private ports as the
//! conformance CLI; it has no native oracle and each script asserts explicit expected values.
use std::env;
use std::error::Error;
use std::path::Path;
use std::process::Command;

#[test]
fn behavior_on_browseros() -> Result<(), Box<dyn Error>> {
    let Ok(browser) = env::var("CLAW_SDK_E2E_BROWSER") else {
        return Ok(());
    };
    let server = Path::new(env!("CARGO_MANIFEST_DIR"));
    let status = Command::new("node")
        .arg(server.join("conformance/conformance.mjs"))
        .arg("behavior")
        .current_dir(server)
        .env("BROWSEROS_APP", browser)
        .env("NEO_BINARY", env!("CARGO_BIN_EXE_browseros-claw-sdk"))
        .status()?;
    assert!(status.success(), "Neo behavior suite failed");
    Ok(())
}
