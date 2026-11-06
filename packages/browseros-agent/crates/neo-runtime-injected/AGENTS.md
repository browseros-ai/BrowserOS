# neo-runtime-injected

The page half of neo-runtime: Playwright's injected script, vendored unchanged, plus Neo's glue table. neo-runtime embeds `dist/*.js` and runs them over CDP in an isolated world of the run's page. Read `../NEO_STYLE.md` before changing anything here.

- `vendor/playwright/` is a byte-for-byte copy of Playwright at the commit `VENDOR.md` names. Never edit it. To upgrade, change `PINNED_COMMIT` in `scripts/sync-vendor.ts` and run `bun run sync-vendor -- --from <playwright checkout at that commit>`.
- `src/glue.ts` holds every page call and `satisfies` neo-runtime-spec's generated `InjectedApi`. It imports vendored code with `import type` only. `src/install.ts` is the one file that value-imports vendored code, and `build.ts` fails on any other.
- `bun run build` writes `dist/`, which neo-runtime embeds. Commit it with every `src/` change and every neo-runtime-spec regeneration (the bundle carries the spec hash). `scripts/neo-check.sh` fails on a stale `dist/`.

Checks: `bun run typecheck`, `bun run test` (vendored bytes against `VENDOR.md`), then `scripts/neo-check.sh` from `packages/browseros-agent`. The browser test loads the built bundle into a real tab. It runs under Node and skips without a browser:

```sh
NEO_E2E_BROWSER="/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo" bun run test:browser
```
