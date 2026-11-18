//! The vendored printer's test bundle neo-playwright-js's build.ts writes beside the cases, run in
//! QuickJS, the engine the TypeScript renderer runs in. `build.ts --check` fails when it is stale,
//! so it is upstream's printer as the repository holds it.

use std::error::Error;
use std::slice;

use rquickjs::{CatchResultExt, Context, Runtime};

use crate::aria_node::AriaNode;

/// Upstream's printer, unmodified: one script that sets the global `renderAriaSnapshotAsYaml`.
const VENDORED_PRINTER: &str = include_str!("../../../tests/data/vendored-printer.js");

/// One QuickJS realm holding the bundle, which sets its one global from inside its own function.
pub(super) struct Bundles(Context);

impl Bundles {
    pub(super) fn new() -> Result<Self, Box<dyn Error>> {
        let runtime = Runtime::new()?;
        let context = Context::full(&runtime)?;
        context.with(|ctx| -> Result<(), String> {
            ctx.eval::<(), _>(VENDORED_PRINTER)
                .catch(&ctx)
                .map_err(|caught| caught.to_string())
        })?;
        Ok(Self(context))
    }

    /// Upstream's own text for `tree`: `renderAriaSnapshotAsYaml(tree)`, its walk recursing.
    pub(super) fn vendored_text(&self, tree: &[AriaNode]) -> Result<String, Box<dyn Error>> {
        self.run(
            &serde_json::to_string(tree)?,
            "renderAriaSnapshotAsYaml(JSON.parse(input))",
        )
    }

    /// Upstream's text for one node: `renderAriaSnapshotAsYaml([node])`.
    pub(super) fn vendored_node_text(&self, node: &AriaNode) -> Result<String, Box<dyn Error>> {
        self.vendored_text(slice::from_ref(node))
    }

    /// Evaluates `script` with `input` as the global string `input`, and reads the string it
    /// returns.
    fn run(&self, input: &str, script: &str) -> Result<String, Box<dyn Error>> {
        Ok(self.0.with(|ctx| -> Result<String, String> {
            ctx.globals()
                .set("input", input)
                .catch(&ctx)
                .map_err(|caught| caught.to_string())?;
            ctx.eval::<String, _>(script)
                .catch(&ctx)
                .map_err(|caught| caught.to_string())
        })?)
    }
}
