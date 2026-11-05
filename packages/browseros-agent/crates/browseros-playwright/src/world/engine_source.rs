//! The page asset, and the one function that installs it with core's seal.

use std::sync::OnceLock;

use browseros_core::sensitive_fields::seal_source;

use crate::probe::PROBE_PROTOCOL;

/// Neo's probe and Playwright's page-side engine in one script, built from the vendored
/// sources by `packages/browseros-playwright-js/scripts/build.ts`.
///
/// Evaluating it in a world defines one global, `__neoProbe`. Its `boot({ frameSeq, epoch })`
/// builds the engine for that document, and refuses to unless core's seal ran first in the
/// same world. The inspector overlay and the `window.playwright` helper are compiled out as
/// empty stand-ins.
const ENGINE_JS: &str = include_str!("../../assets/injected.js");

/// The function an install calls on a world's global with `{ frameSeq, epoch }`. It returns
/// the probe's `described`, whose seq is the one the world keeps, and whose `retired` is the
/// seq a re-key replaced.
///
/// ```text
///  seal_source()                       idempotent per document; before any engine code
///  held = globalThis.__neoProbe
///  held, this protocol   → held.boot(options)   same epoch: no-op; another epoch: re-key
///  held, other protocol  → held.boot(options)   re-keyed first, so it forgets its refs and
///                          delete __neoProbe     their `_ariaRef` expandos, then replaced;
///                                                its reply's `retired` is kept
///  none (or replaced)    → <injected.js>; __neoProbe.boot(options), plus the kept `retired`
/// ```
///
/// The seal runs first in the same call, so a seal that throws leaves no engine behind, and
/// `boot` refuses a world without one: an unsealed engine world cannot exist. The guard on
/// `__neoProbe` keeps one engine per world, because a second copy carries its own module
/// state and would mint colliding refs. Every protocol version keeps `boot` and the
/// `protocol` and `retired` fields of its reply, since a newer build re-keys an older probe
/// through them and must still learn the seq the older one numbered the document with.
pub(crate) fn install_function() -> &'static str {
    static SOURCE: OnceLock<String> = OnceLock::new();
    SOURCE.get_or_init(|| {
        format!(
            "function (options) {{\n{seal};\nconst held = globalThis.__neoProbe;\nlet retired;\n\
             if (held) {{\n  const described = held.boot(options);\n  if (described.protocol \
             === {PROBE_PROTOCOL}) return described;\n  retired = described.retired;\n  delete \
             globalThis.__neoProbe;\n}}\n{ENGINE_JS}\nconst booted = \
             globalThis.__neoProbe.boot(options);\nreturn retired === undefined ? booted : {{ \
             ...booted, retired }};\n}}",
            seal = seal_source(),
        )
    })
}

#[cfg(test)]
mod tests {
    use browseros_core::sensitive_fields::seal_source;

    use super::{ENGINE_JS, install_function};

    const UPSTREAM: &str = "crates/browseros-playwright/third_party/playwright";

    /// The bundler writes one `// <path>` comment per bundled module, relative to
    /// packages/browseros-agent.
    fn bundles_module(path: &str) -> bool {
        ENGINE_JS
            .lines()
            .any(|line| line.trim() == format!("// {path}"))
    }

    #[test]
    fn tests_that_the_engine_defines_only_the_probe_without_the_stubbed_modules() {
        assert!(ENGINE_JS.contains("globalThis.__neoProbe ??= createProbe();"));
        assert!(!ENGINE_JS.contains("__clawInjectedScript"));
        assert!(bundles_module(&format!(
            "{UPSTREAM}/packages/injected/src/injectedScript.ts"
        )));
        assert!(bundles_module(
            "packages/browseros-playwright-js/src/probe/index.ts"
        ));
        for stubbed in ["highlight", "consoleApi"] {
            assert!(
                !bundles_module(&format!("{UPSTREAM}/packages/injected/src/{stubbed}.ts")),
                "{stubbed}.ts is bundled; the build must swap it for its stub"
            );
            assert!(bundles_module(&format!(
                "packages/browseros-playwright-js/src/stubs/{stubbed}.ts"
            )));
        }
    }

    #[test]
    fn tests_that_the_install_seals_before_the_engine_exists() {
        let install = install_function();
        let seal = install.find(seal_source()).unwrap_or(usize::MAX);
        let engine = install.find(ENGINE_JS).unwrap_or(0);
        assert!(
            seal < engine,
            "the seal runs before any engine code, so a throwing seal leaves no engine"
        );
        assert!(
            install.trim_end().ends_with(
                "const booted = globalThis.__neoProbe.boot(options);\nreturn retired === \
                 undefined ? booted : { ...booted, retired };\n}"
            ),
            "the install ends by booting whichever probe is in charge"
        );
    }
}
