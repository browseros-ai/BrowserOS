# Vendored Playwright

Playwright's page-side selector engine, copied unmodified from
[microsoft/playwright](https://github.com/microsoft/playwright) under Apache-2.0. `VERSION`
holds the release tag and its full commit. `LICENSE` and `NOTICE` are upstream's own files.

`neo-playwright` runs this engine in an isolated world so that locators in
Neo's `run` tool resolve exactly as Playwright resolves them: role and accessible names, text
and label matching, CSS, chaining, strict mode and its error messages.

No file here is edited by hand. Each file sits at its upstream path, so a diff against any
Playwright checkout is a plain `diff -r`. `sync.sh` writes `SHA256SUMS`, and the crate's
`tests/engine_sources.rs` fails when a file differs from it or is missing from it.

## What is here

- `packages/injected/src/`. The 12 files of `injectedScript.ts`'s runtime import closure:
  `injectedScript.ts`, `roleUtils.ts`, `roleSelectorEngine.ts`, `selectorEvaluator.ts`,
  `selectorGenerator.ts`, `selectorUtils.ts`, `domUtils.ts`, `layoutSelectorUtils.ts`,
  `xpathSelectorEngine.ts`, `ariaSnapshot.ts`, `ariaSnapshotDistiller.ts`, `utilityScript.ts`.
- `packages/injected/src/bindingsController.ts`. The page side of `exposeBinding`: the
  controller a binding's stub calls, which posts each call to the CDP binding and settles it
  when Neo delivers the agent's answer. Neo builds it into `assets/bindings-controller.js`,
  which runs in the page's main world, as upstream injects it there.
- `packages/isomorphic/`. The 12 shared helpers the engine and the agent runtime use:
  `selectorParser.ts`, `cssParser.ts`, `cssTokenizer.ts`, `locatorUtils.ts`,
  `locatorGenerators.ts`, `stringUtils.ts`, `ariaSnapshot.ts`, `ariaSnapshotRenderer.ts`,
  `yaml.ts`, `utilityScriptSerializers.ts`, `urlMatch.ts`, `mimeType.ts`. The MIME helper
  keeps screenshot filename inference and errors identical to the pinned client. `cssTokenizer.ts` is CC0, from
  tabatkins/parse-css, as its header says.
- `packages/playwright-core/src/server/usKeyboardLayout.ts` and `macEditingCommands.ts`. The US
  keyboard table and the editing commands Playwright sends on macOS, kept as the source of the
  crate's Rust ports (`src/input/us_keyboard_layout.rs` and `src/input/mac_editing_commands.rs`,
  written by `crates/neo-sdk/neo-playwright-js/scripts/port-keyboard-layout.ts`). Both headers
  read "Copyright 2017 Google Inc., modifications copyright Microsoft Corporation", Apache-2.0.
- `packages/protocol/src/serializers.ts`. The protocol's value format, which upstream's client
  uses to serialize an `evaluate` argument (`serializeValue`) and its server uses to read it back
  (`parseSerializedValue`) before re-serializing it for the page. Neo's prelude runs both steps
  in QuickJS, so an argument reaches the page as upstream's does, with upstream's errors.
- `packages/playwright-core/src/client/channels.d.ts` and `packages/protocol/src/structs.d.ts`.
  Upstream's generated protocol declarations, vendored for the prelude's wire check and the
  engine's typecheck. They are types only and never bundled.
- `sync.sh`, `VERSION`, `SHA256SUMS` and this README are ours.

## What the engine build changes

Nothing here is edited, and the page's engine bundle builds every upstream module unmodified;
`crates/neo-sdk/neo-playwright-js/scripts/build.ts` only swaps the inspector overlay and the
devtools helper for stubs.

The agent runtime, `assets/prelude.js`, runs Playwright's value serializers in QuickJS, which
has no `URL`, `Buffer`, `atob` or `btoa`. The build loads `isomorphic/utilityScriptSerializers.ts`
and `protocol/src/serializers.ts` into that bundle only with one import line in front, of Neo's
stand-ins for those four names (`crates/neo-sdk/neo-playwright-js/src/prelude/vendored-globals.ts`),
so they bind as module-scope names and the agent's realm gains no global. The page's bundles
load both files plain.

## The wire contract

The protocol declarations are an independent check on Neo's Rust-owned wire, not a second
implementation of it. Each operation's exhaustive Rust `route()` match binds its upstream
command to a handler. The handler's serde/ts-rs types supply the input, result, brands and
option keys; `neo-playwright/tests/wire_generated.rs` renders them into
`neo-playwright-js/src/prelude/generated/`. The prelude's TypeScript check compares those
generated rows with this checkout's `channels.d.ts` and its referenced `structs.d.ts`.

An intentional difference is explained on the Rust route row (`adds`, `omits`, `retypes`, or
a result exception); Neo-only operations and command replacements have reasons there too.
Changing a field or an upstream declaration therefore reports the affected operation in
`typecheck`. A deviation that no longer applies also fails. Fix the Rust type or the row's
reason, regenerate, and review the resulting diff; never patch a generated declaration or
a vendored file. Policy remains in `OpSpec` and the server's rules, independent of these
shape checks. See `src/op.rs`'s module docs for the complete adding-an-op workflow.

## What is deliberately not here

- `highlight.ts`, `highlight.css` and `consoleApi.ts`. They are the inspector overlay and the
  `window.playwright` devtools helper. They sit in `injectedScript.ts`'s import graph, so the
  engine build swaps them for empty stand-ins through module resolution instead of editing
  upstream code.
- Other type-only imports such as `./selectorEngine`. The bundler erases them, so their files
  are not needed.
- Upstream tests, assets and test-server implementation. The server's
  `../neo-server/conformance/` materializes test-only inputs from this same commit and keeps
  their original assertions and hooks. See that suite's README for coverage and exclusions.
- The Playwright client and server runtime code (apart from the two keyboard tables), and the
  protocol's runtime code (apart from the value serializers). Its two generated type files are
  vendored for the wire check and never bundled. The test runner,
  reporters, the trace viewer, codegen and the recorder, Playwright MCP, browser downloads, and
  every other injected script (`clock.ts`, `webSocketMock.ts`,
  `storageScript.ts`, `webAuthn.ts`, `webMCP.ts`, `bidiInsertText.ts`, `coverageScript.ts`,
  `webview/`).
- Any npm package.

## Upgrading

1. Clone Playwright, or fetch the new release tag into an existing clone.
2. Run `./sync.sh <playwright-checkout> <tag>`. It re-copies the file list, rewrites
   `VERSION` and `SHA256SUMS`, and prints what changed.
3. If upstream added a runtime import, the engine build fails on the unresolved path. Add the
   file to `FILES` in `sync.sh` and to this README, then sync again.
4. Run `bun run typecheck` in `crates/neo-sdk/neo-playwright-js`. It names commands whose
   parameters or results changed, unresolved commands, and stale deviation reasons. Update
   the Rust handler types or route rows, then run
   `cargo test -p neo-playwright --test wire_generated` from `packages/browseros-agent` to
   regenerate (a stale file is rewritten and the test fails; review it and rerun).
5. Rebuild the assets with `bun run build` in `crates/neo-sdk/neo-playwright-js` (Bun 1.4.2,
   the repo's pinned version), then run `bun run build:check` and the crate's engine guards.
   The build also refreshes the probe and wire recordings; review them along with the
   bundles. Run `node scripts/probe-smoke.mjs` there against a throwaway browser.
6. Port the keyboard tables with `bun scripts/port-keyboard-layout.ts` there, and record new
   goldens from the same Playwright release with `scripts/capture-goldens.mjs` (its header says
   how). The crate's keyboard and mouse tests replay those goldens.
7. Update the server's three exact Playwright npm pins to the new release, regenerate its
   conformance inventory, review registration/coverage changes, and run the conformance suite
   on Linux. See `../neo-server/conformance/README.md`.
8. Review the diff and keep release notes in the commit message.
