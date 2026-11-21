//! Guards the rollout boundary while `Behavior` remains constructible by integration tests.

use std::{fs, path::Path};
use syn::buffer::{Cursor, TokenBuffer};

#[test]
fn only_config_reads_the_playwright_run_request() -> anyhow::Result<()> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let config = src.join("config.rs");
    let mut directories = vec![src];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.is_dir() {
                directories.push(path);
            } else if path != config && path.extension().is_some_and(|extension| extension == "rs")
            {
                let tokens = fs::read_to_string(&path)?
                    .parse()
                    .map_err(|error| anyhow::anyhow!("{}: {error}", path.display()))?;
                anyhow::ensure!(
                    !has_request_field_access(TokenBuffer::new2(tokens).begin()),
                    "{} reads the rollout request directly. Use Config::serves_playwright_run().",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

// Inspect tokens inside macro calls too, since a logging field can read the request directly.
fn has_request_field_access(mut cursor: Cursor<'_>) -> bool {
    while let Some((_, next)) = cursor.token_tree() {
        if let Some((operator, after_operator)) = cursor.punct()
            && operator.as_char() == '.'
            && let Some((field, _)) = after_operator.ident()
            && (field == "playwright_run" || field == "r#playwright_run")
        {
            return true;
        }
        if let Some((inner, _, _, _)) = cursor.any_group()
            && has_request_field_access(inner)
        {
            return true;
        }
        cursor = next;
    }
    false
}
