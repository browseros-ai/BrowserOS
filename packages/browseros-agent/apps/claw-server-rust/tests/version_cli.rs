use claw_server_rust::VERSION;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn version_output_is_exact_and_side_effect_free() -> anyhow::Result<()> {
    let root = tempdir()?;
    let state_dir = root.path().join("browserclaw-state");
    let output = Command::new(env!("CARGO_BIN_EXE_browseros-claw-server-rs"))
        .arg("--version")
        .env("BROWSERCLAW_DIR", &state_dir)
        .output()?;

    assert!(
        output.status.success(),
        "version command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, format!("{VERSION}\n").as_bytes());
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!state_dir.exists());
    Ok(())
}
