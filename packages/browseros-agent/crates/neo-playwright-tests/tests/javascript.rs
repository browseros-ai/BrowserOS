//! The default Cargo gate runs the actual JS transport and codec checks under both hosts.
use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn javascript_transport_and_value_contracts() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("js");
    let mut files: Vec<_> = fs::read_dir(&directory)
        .expect("harness JS directory")
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| path.to_string_lossy().ends_with(".test.mjs"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "the JS contract gate must not become empty"
    );
    for host in ["node", "bun"] {
        let mut command = Command::new(host);
        if host == "node" {
            command.args(["--test", "--test-concurrency=1"]);
        } else {
            command.args(["test", "--timeout", "30000"]);
        }
        command.env_remove("NEO_OBSERVER_BROWSER");
        let output = command
            .args(&files)
            .output()
            .expect("installed repository JS host");
        assert!(
            output.status.success(),
            "{host} contract checks failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
