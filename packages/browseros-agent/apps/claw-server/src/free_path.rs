//! Golden copies of what an agent without ultrafast reads: instructions, tools and the guide.
//! pursue steering is written for the ultrafast path only, so these must not move when it does.
//! Run the tests with `BLESS_FREE_PATH=1` to rewrite the copies after a deliberate change.

use std::{error::Error, fs, path::PathBuf};

pub(crate) fn assert_unchanged(name: &str, actual: &str) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/free_path")
        .join(name);
    if std::env::var_os("BLESS_FREE_PATH").is_some() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, actual)?;
        return Ok(());
    }
    let expected = fs::read_to_string(&path)?;
    assert!(
        expected == actual,
        "{name} changed for agents without ultrafast; if that is intended, rerun with BLESS_FREE_PATH=1 and review the diff"
    );
    Ok(())
}
