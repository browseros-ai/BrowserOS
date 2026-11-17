//! Raw SQL is confined to file pragmas and damaged-file recovery; row writes stay in storage.

use std::error::Error;
use std::fs;
use std::path::Path;

fn inspect(
    root: &Path,
    directory: &Path,
    violations: &mut Vec<String>,
) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            inspect(root, &path, violations)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let relative = path.strip_prefix(root)?.to_string_lossy();
            let source = fs::read_to_string(&path)?;
            let raw_allowed = matches!(
                relative.as_ref(),
                "storage/trace_migrator.rs" | "salvage.rs" | "storage/test_faults.rs"
            );
            for token in [
                "execute_unprepared",
                "Statement::from_",
                "sqlx::query",
                "raw_sql",
                "Expr::cust",
            ] {
                if !raw_allowed && source.contains(token) {
                    violations.push(format!("{relative}: raw SQL escape {token}"));
                }
            }
            for token in ["update_many", "delete_many", "::insert(", "insert_many"] {
                if !relative.starts_with("storage/") && source.contains(token) {
                    violations.push(format!("{relative}: row-write escape {token}"));
                }
            }
        }
    }
    Ok(())
}

#[test]
fn storage_owns_raw_escapes_and_every_row_write() -> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();
    inspect(&root, &root, &mut violations)?;
    assert!(
        violations.is_empty(),
        "storage ownership violations:\n{}",
        violations.join("\n")
    );
    Ok(())
}
