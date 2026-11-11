//! Playwright-compatible locators and actions for BrowserOS Neo's `run` tool (claw-sdk).
//!
//! Agent scripts call the Playwright API they already know. Element work (selector resolution,
//! strict mode, actionability states, hit tests) runs in the page's sealed isolated world
//! (`browseros-sdk`), using Playwright's own `InjectedScript` built from the sources vendored
//! unmodified under `third_party/playwright/`; the build only has the engine mask field values
//! out of page text before it cuts or quotes that text (its README says where). Retries,
//! timeouts, frame hops and every decision stay in Rust, which also sends the trusted input.
//! The isolated world shares the page's DOM but not its JavaScript, so the page can neither see
//! nor tamper with the engine.
//!
//! Licensing: Playwright is Apache-2.0. `THIRD_PARTY_NOTICES.md` carries its `LICENSE` and
//! `NOTICE` and must ship next to any binary that embeds this crate.
//!
//! The host sees one facade and one op table. Each Playwright call the prelude makes is one
//! [`Op`] (`locator.click`, `page.ariaSnapshot`, ...); the host registers every op from
//! [`Op::all`] and hands each call to [`Playwright::invoke`] with an [`OpEnv`] for that call.
//! Each op's [`OpSpec`] declares the [`Capability`]s it reaches beyond its page's sealed world
//! (the page's main world, its pixels, Neo's screen, Neo's disk), and the host holds and polices
//! ops by those declarations, never by op name. Each capability's gateway in the crate refuses
//! an op whose spec does not declare it.
//!
//! ```text
//!  agent JS ─ prelude ─ __neo.invoke(op, json) ─ host dispatcher (policy, trace, delivery)
//!                                                        │ OpEnv: recheck, mark_sent, note
//!  Playwright::invoke(env, op, json) ◀───────────────────┘
//!    └─ OpScope (call header) ─ family leaf (its own typed input) ─ sealed world per frame
//! ```
//!
//! Ownership: state that outlives one op belongs to [`Playwright`] (per browser connection) or
//! to the host's [`RunState`] (per run, dropped with it). An op's own state lives in its scope
//! and ends with it, so nothing else is shared between operations.

mod action;
mod backoff;
mod call_log;
mod download_facts;
mod events;
mod expect;
mod input;
mod listed_enum;
mod locator_handler;
mod object_group;
mod op;
mod op_env;
mod op_input;
mod op_output;
mod op_scope;
mod op_spec;
mod page;
mod playwright;
mod playwright_error;
mod playwright_hooks;
mod prelude_source;
mod probe;
mod read;
mod resolve;
mod run_state;
mod snapshot;
mod target;
#[cfg(test)]
mod test_env;
#[cfg(test)]
mod test_support;
mod world;

pub use download_facts::{DownloadFacts, DownloadState};
pub use op::Op;
pub use op_env::{OpEnv, Refusal};
pub use op_spec::{Capability, OpLifetime, OpSpec, OpTarget};
pub use playwright::Playwright;
pub use playwright_error::{Answer, ErrorCode, PlaywrightError, TimedOut};
pub use playwright_hooks::{PlaywrightHooks, TypedBy};
pub use prelude_source::PRELUDE_JS;
pub use run_state::RunState;

#[cfg(test)]
mod tests {
    use super::PRELUDE_JS;

    const LICENSE: &str = include_str!("../third_party/playwright/LICENSE");
    const NOTICE: &str = include_str!("../third_party/playwright/NOTICE");
    const NOTICES: &str = include_str!("../THIRD_PARTY_NOTICES.md");
    const UPSTREAM: &str = "crates/neo-sdk/neo-playwright/third_party/playwright";

    /// The bundler writes one `// <path>` comment per bundled module, relative to
    /// packages/browseros-agent.
    fn bundles_module(asset: &str, path: &str) -> bool {
        asset
            .lines()
            .any(|line| line.trim() == format!("// {path}"))
    }

    #[test]
    fn tests_that_the_prelude_builds_neos_classes_on_the_vendored_helpers_and_no_engine() {
        assert!(
            bundles_module(
                PRELUDE_JS,
                "crates/neo-sdk/neo-playwright-js/src/prelude/index.ts"
            ),
            "the prelude asset is built from the prelude's own entry"
        );
        for helper in [
            "selectorParser",
            "locatorUtils",
            "locatorGenerators",
            "urlMatch",
        ] {
            assert!(
                bundles_module(
                    PRELUDE_JS,
                    &format!("{UPSTREAM}/packages/isomorphic/{helper}.ts")
                ),
                "the prelude must bundle the vendored {helper}.ts"
            );
        }
        assert!(
            !bundles_module(
                PRELUDE_JS,
                &format!("{UPSTREAM}/packages/injected/src/injectedScript.ts")
            ),
            "the agent runtime has no DOM; the page engine must stay out of it"
        );
    }

    #[test]
    fn tests_that_the_notices_carry_playwrights_license_and_notice_verbatim() {
        // Upstream's LICENSE uses CRLF; the Markdown notices file is LF throughout.
        assert!(NOTICES.contains(&LICENSE.replace("\r\n", "\n")));
        assert!(NOTICES.contains(NOTICE));
        assert!(NOTICES.contains("cssTokenizer.ts"));
        assert!(NOTICES.contains("usKeyboardLayout.ts"));
        assert!(NOTICES.contains("macEditingCommands.ts"));
    }
}
