//! The two test bundles neo-playwright-js's build.ts writes beside the cases, run in QuickJS, the
//! engine the TypeScript renderer runs in. `build.ts --check` fails when either is stale, so they
//! are today's renderer and the vendored printer as the repository holds them.

use std::error::Error;
use std::slice;

use rquickjs::{CatchResultExt, Context, Runtime};
use serde_json::json;

use crate::aria_node::AriaNode;
use crate::snapshot::Snapshot;

/// Today's TypeScript renderer: one script that sets the global `todaysAnswer(call, result)`
/// (neo-playwright-js `scripts/snapshot-cases/answer.ts`), the dispatch the case writer uses.
const TODAYS_RENDERER: &str = include_str!("../../tests/data/todays-renderer.js");

/// Upstream's printer, unmodified: one script that sets the global `renderAriaSnapshotAsYaml`.
const VENDORED_PRINTER: &str = include_str!("../../tests/data/vendored-printer.js");

/// One QuickJS realm holding both bundles. Each sets its one global from inside its own function,
/// so they share nothing else.
pub(crate) struct Bundles(Context);

impl Bundles {
    pub(crate) fn new() -> Result<Self, Box<dyn Error>> {
        let runtime = Runtime::new()?;
        let context = Context::full(&runtime)?;
        context.with(|ctx| -> Result<(), String> {
            for script in [TODAYS_RENDERER, VENDORED_PRINTER] {
                ctx.eval::<(), _>(script)
                    .catch(&ctx)
                    .map_err(|caught| caught.to_string())?;
            }
            Ok(())
        })?;
        Ok(Self(context))
    }

    /// Upstream's own text for `tree`: `renderAriaSnapshotAsYaml(tree)`, its walk recursing.
    pub(crate) fn vendored_text(&self, tree: &[AriaNode]) -> Result<String, Box<dyn Error>> {
        self.run(
            &serde_json::to_string(tree)?,
            "renderAriaSnapshotAsYaml(JSON.parse(input))",
        )
    }

    /// Upstream's text for one node: `renderAriaSnapshotAsYaml([node])`.
    pub(crate) fn vendored_node_text(&self, node: &AriaNode) -> Result<String, Box<dyn Error>> {
        self.vendored_text(slice::from_ref(node))
    }

    /// Today's Playwright-format text for `snapshot`: `todaysAnswer({ playwright: {} }, result)`,
    /// the result being the tree and the frames not read as Rust hands them over today.
    pub(crate) fn todays_playwright_text(
        &self,
        snapshot: &Snapshot,
    ) -> Result<String, Box<dyn Error>> {
        let partial: Vec<_> = snapshot
            .partial
            .iter()
            .map(
                |frame| json!({ "ref": frame.r#ref, "frame": frame.frame, "reason": frame.reason }),
            )
            .collect();
        let result = json!({ "json": snapshot.tree, "partial": partial });
        self.run(
            &result.to_string(),
            "todaysAnswer({ playwright: {} }, JSON.parse(input)).text",
        )
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
