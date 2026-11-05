//! The agent-side half of the crate: the script claw-sdk installs so agents can write
//! Playwright.

/// Neo's Playwright prelude: the client API agent scripts call, built from
/// `packages/browseros-playwright-js/src/prelude` and the vendored isomorphic helpers by
/// `packages/browseros-playwright-js/scripts/build.ts`.
///
/// claw-sdk's runtime evaluates it in each run's fresh QuickJS context, after its bootstrap
/// and before the agent's script (sdk-skeleton D6, step 4), so its state lives for one run.
/// It defines the `page`, `context`, `browser` and `expect` globals and reaches Rust only
/// through the bootstrap's `__neo.invoke`, one call per [`Op`](crate::Op), whose input
/// [`Playwright::invoke`](crate::Playwright::invoke) decodes. It names no global outside
/// ECMAScript's own, so QuickJS runs it as built. What it assumes of the bootstrap beyond
/// sdk-skeleton D8 is listed in `src/prelude/bridge.ts`.
pub const PRELUDE_JS: &str = include_str!("../assets/prelude.js");
