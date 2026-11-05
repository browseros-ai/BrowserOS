# Vendored Playwright

Playwright's page-side selector engine, copied unmodified from
[microsoft/playwright](https://github.com/microsoft/playwright) under Apache-2.0. `VERSION`
holds the release tag and its full commit. `LICENSE` and `NOTICE` are upstream's own files.

`browseros-playwright` runs this engine in a sealed isolated world so that locators in
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
- `packages/isomorphic/`. The 11 shared helpers the engine and the agent runtime use:
  `selectorParser.ts`, `cssParser.ts`, `cssTokenizer.ts`, `locatorUtils.ts`,
  `locatorGenerators.ts`, `stringUtils.ts`, `ariaSnapshot.ts`, `ariaSnapshotRenderer.ts`,
  `yaml.ts`, `utilityScriptSerializers.ts`, `urlMatch.ts`. `cssTokenizer.ts` is CC0, from
  tabatkins/parse-css, as its header says.
- `packages/playwright-core/src/server/usKeyboardLayout.ts` and `macEditingCommands.ts`. The US
  keyboard table and the editing commands Playwright sends on macOS, kept as the source of the
  crate's Rust ports (`src/input/us_keyboard_layout.rs` and `src/input/mac_editing_commands.rs`,
  written by `packages/browseros-playwright-js/scripts/port-keyboard-layout.ts`). Both headers
  read "Copyright 2017 Google Inc., modifications copyright Microsoft Corporation", Apache-2.0.
- `sync.sh`, `VERSION`, `SHA256SUMS` and this README are ours.

## What is deliberately not here

- `highlight.ts`, `highlight.css` and `consoleApi.ts`. They are the inspector overlay and the
  `window.playwright` devtools helper. They sit in `injectedScript.ts`'s import graph, so the
  engine build swaps them for empty stand-ins through module resolution instead of editing
  upstream code.
- Type-only imports such as `./selectorEngine` and `@protocol/structs`. The bundler erases
  them, so their files are not needed.
- The Playwright client, server and protocol (apart from the two keyboard tables), the test runner,
  reporters, the trace viewer, codegen and the recorder, Playwright MCP, browser downloads, and
  every other injected script (`clock.ts`, `bindingsController.ts`, `webSocketMock.ts`,
  `storageScript.ts`, `webAuthn.ts`, `webMCP.ts`, `bidiInsertText.ts`, `coverageScript.ts`,
  `webview/`).
- Any npm package.

## Upgrading

1. Clone Playwright, or fetch the new release tag into an existing clone.
2. Run `./sync.sh <playwright-checkout> <tag>`. It re-copies the file list, rewrites
   `VERSION` and `SHA256SUMS`, and prints what changed.
3. If upstream added a runtime import, the engine build fails on the unresolved path. Add the
   file to `FILES` in `sync.sh` and to this README, then sync again.
4. Rebuild the assets with `bun run build` in `packages/browseros-playwright-js` (Bun 1.4.2,
   the repo's pinned version). Its test fails while `crates/browseros-playwright/assets/`
   or `crates/browseros-playwright/tests/data/probe-protocol.json` is stale. Then run
   `node scripts/probe-smoke.mjs` there against a throwaway browser.
5. Port the keyboard tables with `bun scripts/port-keyboard-layout.ts` there, and record new
   goldens from the same Playwright release with `scripts/capture-goldens.mjs` (its header says
   how). The crate's keyboard and mouse tests replay those goldens.
6. Review the diff and keep release notes in the commit message.
