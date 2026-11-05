//! Keeps the prelude's op list, `packages/browseros-playwright-js/src/prelude/ops.generated.ts`,
//! equal to Rust's op table (`Op::all()`). A stale file is rewritten and the test fails, so the
//! regenerated file is reviewed and committed like any other change.

use std::error::Error;
use std::fmt::Write as _;
use std::io::ErrorKind;
use std::path::Path;
use std::{env, fs};

use browseros_playwright::{Op, OpLifetime, OpTarget};
use browseros_trace::Effect;

/// From this crate's manifest directory.
const GENERATED: &str = "../../packages/browseros-playwright-js/src/prelude/ops.generated.ts";

/// The TypeScript module, in the shape Biome formats it (single quotes, no semicolons, one
/// expanded object per op), so `bun run check` leaves the generated file alone.
fn render() -> Result<String, Box<dyn Error>> {
    let mut out = String::from(
        "// Generated from browseros-playwright's op table (`Op::all()`). Do not edit: change the\n\
         // op's family file in Rust, then run\n\
         // `cargo test -p browseros-playwright --test ops_generated`, which rewrites this file.\n\
         \n\
         /**\n \
         * Every operation the prelude can invoke, in Rust's registration order, with what the\n \
         * host's dispatcher knows about it: whether it may change the page, which input field\n \
         * holds the page handle, and whether it is the background event pump.\n \
         */\n\
         export const OPS = {\n",
    );
    for op in Op::all() {
        let spec = op.spec();
        let effect = match spec.effect {
            Effect::Read => "'read'",
            Effect::Write => "'write'",
        };
        let target = match spec.target {
            OpTarget::None => "null".to_owned(),
            OpTarget::Field(field) => format!("'{field}'"),
        };
        let lifetime = match spec.lifetime {
            OpLifetime::Operation => "'operation'",
            OpLifetime::Background => "'background'",
        };
        write!(
            out,
            "  '{}': {{\n    effect: {effect},\n    target: {target},\n    lifetime: {lifetime},\n  }},\n",
            spec.name
        )?;
    }
    out.push_str(
        "} as const\n\
         \n\
         /** A name `__neo.invoke` accepts for a Playwright operation. */\n\
         export type OpName = keyof typeof OPS\n",
    );
    Ok(out)
}

#[test]
fn tests_that_the_prelude_op_list_matches_the_rust_op_table() -> Result<(), Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(GENERATED);
    let fresh = render()?;
    let committed = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    if committed != fresh {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &fresh)?;
        panic!(
            "{} did not match Op::all() and has been rewritten; review and commit it",
            path.display()
        );
    }
    Ok(())
}

#[test]
fn tests_that_names_are_safe_in_a_single_quoted_string() {
    for op in Op::all() {
        let name = op.spec().name;
        assert!(
            name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.'),
            "{name} is written into TypeScript between single quotes"
        );
    }
}
