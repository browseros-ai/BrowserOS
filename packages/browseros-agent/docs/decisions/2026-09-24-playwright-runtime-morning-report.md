# Morning report: `playwright` tool for BrowserOS neo

Branch: `feat/playwright-runtime` (worktree `.wt/feat/playwright-runtime`),
53 commits on top of `main` (8d9a40820), not pushed. Everything below is on
that branch. All documents: `~/llm/code/browseros-project/grove-ref/main-1/playwright-runtime/`
(this dir; `STATUS.md` is the full night log, `DECISION.md` the contract,
`briefs/`, `design/`, `spikes/`, `conformance/`, `reports/`).

## 1. What works

Agents can now call a new MCP tool **`playwright`** and send ordinary
Playwright JavaScript (`context`, `page`, locators, `expect`, plus a `neo`
namespace for BrowserOS extras). It runs in-process in the existing QuickJS
runtime; locators, strict mode, element states and web-first `expect` are
**Playwright's own injected script** (1.63.0, Apache-2.0, vendored, pinned by
SHA) evaluated in an isolated world over the server's existing CDP session;
trusted input, navigation, dialogs, downloads, popups, screenshots and PDF are
`browseros-core`. Nothing new ships (no Node, no sidecar, no Chromium change);
the binary grows by the ~320 KB bundle. `run` and every granular tool are
untouched and still pass their suites.

Every guarantee is inherited, not re-implemented: each Playwright call is a
child audit row under the script dispatch with Playwright's method name
(`page.goto`, `locator.click`, `expect.toBeVisible`), per-step screenshots,
page claiming and one tab group per session before `newPage()` resolves,
ownership as a label with notices, replay attribution, cancellation and the
30 s cap. Typed secrets (password / secret-autocomplete inputs) are masked in
child rows, in the parent script text and in `expect` diagnostics.

Verified tonight (all against isolated BrowserOS neo instances, never your
live browser):

| Check | Result | Command |
|---|---|---|
| Rust gates (build, core/mcp/server tests, clippy `-D warnings`, fmt) | green | `cd packages/browseros-agent && cargo test -p browseros-mcp --locked` (167+ tests) |
| TypeScript gate | green | `bun run check` |
| Full real-browser conformance suite incl. 15 new Playwright cases | **150 pass / 0 fail** | `BROWSEROS_BINARY='/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo' bun contracts/claw-mcp/tests/run.ts` |
| Mediator's own drive: strict-mode error text, disambiguation, example.com live, `neo.*` from a script, legacy `run` in the same session, two concurrent sessions isolated, 30 s cap, nested audit rows + screenshots, one tab group per session | **10/10** | `BROWSEROS_BINARY=… bun ~/llm/code/browseros-project/grove-ref/main-1/playwright-runtime/reports/mediator-drive.ts` |
| Cockpit: nested steps under the script row, highlighted code with the password already redacted, child error badges, per-tab context, replay captions | **verified by eye** on the real product loop with S01/S07/F13 sessions (screenshots in `reports/w4-cockpit/`); 445 claw-app tests | `bun run --filter @browseros/claw-app test` |

To try it yourself: `cd packages/browseros-agent && bun run dev:claw:watch:new`
(launches the installed neo with the dev cockpit and this server), connect
your agent to the printed `/mcp` endpoint, and send:

```js
await page.goto('https://news.ycombinator.com');
await page.getByRole('link', { name: 'new', exact: true }).click();
await expect(page).toHaveURL(/newest/);
return (await page.locator('.titleline > a').allInnerTexts()).slice(0, 10);
```

The tool description, the MCP instructions and both skill copies
(`resources/skills/browserclaw/SKILL.md`, `skills/browseros-neo/SKILL.md`)
now teach `playwright` first, `run` for saved helpers and raw CDP, granular
tools as the fallback.

## 2. What does not work / known gaps

- Not available by design (throw `Error: not available in BrowserOS neo: <api>. <hint>`):
  `route`/`unroute`, `request`, `cookies`/`storageState`, `addInitScript`,
  `setViewportSize`, `emulateMedia`, `tracing`, `video`, `exposeFunction`,
  `page.pause`. `browser.newContext()` returns the one signed-in context
  with a warning; `bringToFront()` is a no-op (agents never steal focus).
- Saved helpers (`helpers.*`) are `run`-only; no Playwright-dialect helpers yet.
- Transformed (scaled/rotated) iframes are not handled in hit testing
  (Playwright's simple path only). Cross-origin iframes work.
- The facade enables `Emulation.setFocusEmulationEnabled` on a page session
  the first time the engine acts on it (as Playwright does): the page then
  reports itself visible/focused while it stays a background tab.
- Only macOS arm64 was exercised; the vendored bundle is plain JS so other
  targets should be unaffected, but no cross-target build was run.
- Wave 4 cockpit pass landed and is merged (readability fixes: full-viewport
  previews, wrapped selectors, bounded code panels, Failed badges, more
  replay captions). Wave 4 hardening corpus (S16–S25) may still be running
  or have landed after this report; see the end of `STATUS.md`.

## 3. What you decided for me (overturn any in 30 seconds)

- DECIDED engine: Playwright's vendored injected script over our own CDP
  session, in-process. Sidecar with real `playwright-core` (design B) lost on
  +64 MB per platform, Bun/Playwright pin with source rewrites, a CDP relay
  that fakes `Target.setAutoAttach`, per-platform signing, and `node:vm` not
  being a boundary (escape demonstrated). rustwright in-process (design A)
  lost on fidelity: locator specs public for click only, no public waits,
  `>>`/`:has-text`/`nth=` broken, own Tokio runtime, `pages()` attaches to
  every user tab, alpha code. Both remain documented fallbacks (`design/`).
- DECIDED a new tool named `playwright` beside `run` (not `run` + `api`).
- DECIDED no Chromium change; new tabs go through the fork's
  `Browser.createTab background:true`. Follow-up flagged: default
  `Target.createTarget` to background under the never-steal-focus pref so
  any third-party CDP client is safe.
- DECIDED `context` ⇔ agent session; `context.pages()` = own tabs (sync);
  `neo.pages({ownership:'all'})` for everyone's; lazy `page` global.
- DECIDED action default 10 s, navigation 10 s, `expect` 5 s, inside the
  unchanged 30 s cap; no job model overnight.
- DECIDED reuse `InnerCallHook` through the shared bridge; the
  `ScriptHost`/`ScriptDispatch` seam from the architecture scan
  (`design/host-seam.md`) is deferred until a second engine exists.
- DECIDED redaction of typed secrets in audit rows (fail closed), and
  `expect` diagnostics carry only `received.value` (an `ariaSnapshot` was
  observed to leak a password field).
- DECIDED facade bookkeeping reads (`page.info`, `context.lastPage`) are not
  audit rows; explicit `page.title()`/`page.content()` are.
- DECIDED workers never pushed to `origin`; merging into the integration
  branch was done locally with `--no-ff` merges per piece.

## 4. What is left (priority order)

1. Read the wave-4 hardening result at the end of `STATUS.md`; merge
   `feat/pw-p4-actions` if it landed after this report, re-run the full
   suite, then push `feat/playwright-runtime` and open the PR yourself (I
   did not push).
2. Chromium follow-up: background default for `Target.createTarget` under
   the pref (one patch in `chromium_patches/content/browser/devtools/protocol/target_handler.cc`).
3. Playwright-dialect saved helpers (`design/host-seam.md` §HelperDialect).
4. `ToolMetadata.kind` to stop name-sniffing (`ARBITRARY_SCRIPT_TOOLS`,
   `REPORTED_TOOLS`, cockpit `NOTABLE_TOOLS`) — architecture scan candidate 3.
5. Product docs page under `docs/neo/mcp/` for the new tool.
6. Cleanup of the 15 `pw-*` worktrees/branches and the `ft` session panes
   (left for you per the standing orders).

## 5. What I could not verify

- Behaviour on Windows/Linux builds and the release packaging
  (`scripts/build/claw-server-rust.ts`); no cross-target build was run.
- Long-running real sites beyond example.com and the local fixtures.
- Replay video of a playwright session (recorder injection is unchanged and
  keyed on the same claims, so it should work; not watched).
