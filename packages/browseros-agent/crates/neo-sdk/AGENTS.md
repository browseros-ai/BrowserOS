# neo-sdk

The Playwright-shaped objects (`Page`, `Frame`, `Locator`, `BrowserContext`) an agent script sees inside neo-runtime's QuickJS realm. Read `../NEO_STYLE.md` before changing anything here.

- Runs in QuickJS: ES2023 only, no DOM or Node APIs, and the bundle defines no globals.
- Depends on nothing at runtime; `src/runtime-bridge.ts` is its only link to Rust.
- `bun run build` writes `dist/sdk.js`, which neo-runtime embeds. Commit it with every `src/` change; `scripts/neo-check.sh` fails on a stale bundle.
- Locators are Playwright's: `src/locator.ts` keeps Playwright's method bodies, and every selector comes from `vendor/playwright/`, an unchanged copy at Playwright 26137face. Never edit vendored files; `bun run test` re-hashes them against `vendor/playwright/VENDOR.md`. `tests/goldens/` holds Playwright's recorded selector strings, which neo-runtime's tests compare against.

- Start each ported file's header with `Playwright: \`client/<file>.ts\` \`<Class>\``, list every difference as a `Deviation:` line, and start Neo's own files with `Neo:` (`../NEO_STYLE.md`, Provenance). neo-runtime's `cargo test --test provenance` checks the lines.

Checks: `bun run typecheck` and `bun run test`, then `scripts/neo-check.sh` from `packages/browseros-agent`.

The parity tests (`tests/parity/`) run the same scripts through real Playwright and through Neo on one browser and compare what each script and each page saw: `page-url.test.mjs` runs `tests/scripts/watch-url.js` on one tab; `click.test.mjs` runs each click case on the pages in `tests/pages/`, on fresh tabs, and compares the script's result, the error's kind and the events each page saw; never the error's wording, the call log or retry timing. `pages`, `newPage`, `close` and `bringToFront.test.mjs` each run their `tests/scripts/` script on fresh `tests/pages/tab.html` tabs, Playwright's turn and then Neo's, and compare the values and which tabs each left open, closed or active; Neo opens `newPage` in the background, and its `bringToFront` selects the tab where Playwright's leaves headless BrowserOS's selection as it was, both asserted. `innerText` and `getAttribute.test.mjs` each run one script through `compareMethod` on fresh `tests/pages/element-text.html` pages and compare its values, their types and the rejections' kinds, and that each page kept its markup and focus. `isVisible` and `isChecked.test.mjs` compare booleans, matching and error categories across Locator, Page and Frame receivers. `harness.mjs` holds what they share, and imports playwright-core itself so colours are off first. The frame tests (`frameLocator`, `contentFrame` and `frames.test.mjs`, and a frame test in each method file) run on `tests/pages/frame-top.html`, a tree of same-origin, cross-site out-of-process and nested frames, through `frames.mjs`, which reads every frame's effect so a call routed to the wrong frame shows as a changed decoy; `NEO_PARITY_REFERENCE_ONLY=1` runs only their Playwright half. They run under Node (Playwright's `connectOverCDP` hangs under Bun), one file at a time, and skip without a browser:

```sh
NEO_E2E_BROWSER="/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo" bun run test:parity
```
