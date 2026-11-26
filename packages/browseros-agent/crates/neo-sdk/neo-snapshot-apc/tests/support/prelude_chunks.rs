//! Selector chunks built the way scripts build them: neo-playwright's bundled prelude, run in
//! QuickJS, composes a locator with its own `Locator` class and splits it with Playwright 1.64.1's
//! `splitSelectorByFrame`, so a test's chunk is byte for byte what the bridge would carry.

use std::error::Error;
use std::fs;
use std::path::Path;

use neo_snapshot_apc::select::ParsedSelector;
use rquickjs::{CatchResultExt, Context, Runtime};

/// One QuickJS context holding the prelude, with a `page` whose `locator`, `getByRole` and `$`
/// return what the real page object returns for one frame.
pub(crate) struct Prelude {
    _runtime: Runtime,
    context: Context,
}

impl Prelude {
    /// Evaluates the prelude with its host bridge stubbed out, exposing `page` and the two
    /// helpers the tests read.
    pub(crate) fn load() -> Result<Self, Box<dyn Error>> {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../neo-playwright/assets/prelude.js");
        let source = fs::read_to_string(&path)?;
        // The prelude is one IIFE; the test bindings go inside it, where `Locator`,
        // `getByRoleSelector`, `splitSelectorByFrame` and `asLocator` are in scope.
        let body = source
            .trim_end()
            .strip_suffix("})();")
            .ok_or("prelude.js no longer ends with `})();`")?;
        let script = format!(
            "globalThis.__neo = {{ brands: {{ register() {{}} }}, onReply() {{}}, onSettle() {{}}, provide() {{}} }};\n\
             {body}\n\
             const __frame = {{}};\n\
             globalThis.page = {{\n\
               locator: (selector) => new Locator(__frame, selector),\n\
               getByRole: (role, options) => new Locator(__frame, getByRoleSelector(role, options)),\n\
               $: (selector) => selector,\n\
             }};\n\
             globalThis.__select = {{ splitSelectorByFrame, asLocator }};\n\
             }})();"
        );
        let runtime = Runtime::new()?;
        let context = Context::full(&runtime)?;
        context.with(|ctx| {
            ctx.eval::<(), _>(script.as_str())
                .catch(&ctx)
                .map_err(|caught| caught.to_string())
        })?;
        Ok(Self {
            _runtime: runtime,
            context,
        })
    }

    /// The one chunk of `locator`, a JavaScript expression over `page` that yields a `Locator`
    /// or a selector string, such as `page.getByRole('button').nth(1)`.
    pub(crate) fn chunk(&self, locator: &str) -> Result<ParsedSelector, Box<dyn Error>> {
        let chunks: Vec<ParsedSelector> = serde_json::from_str(&self.eval(&format!(
            "JSON.stringify(__select.splitSelectorByFrame(__selectorOf({locator})).chunks)"
        ))?)?;
        let [chunk] = <[ParsedSelector; 1]>::try_from(chunks)
            .map_err(|chunks| format!("{locator} split into {} chunks", chunks.len()))?;
        Ok(chunk)
    }

    /// `locator` as upstream's `asLocator('javascript', …)` prints it, the text a strict-mode
    /// violation names.
    pub(crate) fn locator_text(&self, locator: &str) -> Result<String, Box<dyn Error>> {
        self.eval(&format!(
            "__select.asLocator('javascript', __selectorOf({locator}))"
        ))
    }

    fn eval(&self, expression: &str) -> Result<String, Box<dyn Error>> {
        let script = format!(
            "(() => {{ const __selectorOf = (value) => typeof value === 'string' ? value : value._selector; return {expression}; }})()"
        );
        let output = self.context.with(|ctx| {
            ctx.eval::<String, _>(script.as_str())
                .catch(&ctx)
                .map_err(|caught| caught.to_string())
        })?;
        Ok(output)
    }
}
