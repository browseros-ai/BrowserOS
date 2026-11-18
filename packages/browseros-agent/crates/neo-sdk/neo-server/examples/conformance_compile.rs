//! Compiles generated conformance programs in the same QuickJS engine the server embeds.
//! No program is executed: missing SDK globals cannot hide syntax errors in a selected spec.
use std::error::Error;
use std::{env, fs};

use rquickjs::{Context, Runtime, Value};

// Read the same API parameter declarations the production prelude supplies. A bare Function
// parser would miss an invalid `const page` redeclaration in the real run wrapper.
fn parameters() -> String {
    let sources = [
        include_str!("../src/runtime/js/neo_prelude.js"),
        include_str!("../../neo-playwright-js/src/prelude/index.ts"),
    ];
    sources
        .iter()
        .flat_map(|source| source.split("provide(").skip(1))
        .filter_map(|tail| {
            let tail = tail.trim_start();
            let quote = tail.chars().next()?;
            if !matches!(quote, '\'' | '"') {
                return None;
            }
            tail.get(1..)?.split(quote).next()
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn main() -> Result<(), Box<dyn Error>> {
    let directory = env::args().nth(1).ok_or("expected a programs directory")?;
    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "js") {
            let body = format!(
                "return async function({}) {{\n{}\n}}",
                parameters(),
                fs::read_to_string(&path)?
            );
            let source = format!("new Function({})", serde_json::to_string(&body)?);
            context
                .with(|ctx| ctx.eval::<Value<'_>, _>(source).map(|_compiled| ()))
                .map_err(|error| format!("{}: {error}", path.display()))?;
        }
    }
    Ok(())
}
