# neo-sdk

The Playwright-shaped objects (`Page`, `Frame`, `Locator`, `BrowserContext`) an agent script sees inside neo-runtime's QuickJS realm. Read `../NEO_STYLE.md` before changing anything here.

- Runs in QuickJS: ES2023 only, no DOM or Node APIs, and the bundle defines no globals.
- Depends on nothing at runtime; `src/runtime-bridge.ts` is its only link to Rust.
- `bun run build` writes `dist/sdk.js`, which neo-runtime embeds. Commit it with every `src/` change; `scripts/neo-check.sh` fails on a stale bundle.
- Locators are Playwright's: `src/locator.ts` keeps Playwright's method bodies, and every selector comes from `vendor/playwright/`, an unchanged copy at Playwright 26137face. Never edit vendored files; `bun run test` re-hashes them against `vendor/playwright/VENDOR.md`. `tests/goldens/` holds Playwright's recorded selectors and clicks, which neo-runtime's tests compare against.

Checks: `bun run typecheck` and `bun run test`, then `scripts/neo-check.sh` from `packages/browseros-agent`.

The parity test runs `tests/scripts/watch-url.js` through real Playwright and through Neo on the same tab and compares the results. It runs under Node (Playwright's `connectOverCDP` hangs under Bun) and skips without a browser:

```sh
NEO_E2E_BROWSER="/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo" bun run test:parity
```
