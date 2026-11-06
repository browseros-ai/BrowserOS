# Playwright click and selector goldens for Neo

Recorded truth for Neo's locator and click work, captured before any Neo code exists. Neo tests compare against these files instead of hand-typed expectations.

## Files

| File | Contents |
| --- | --- |
| `capture-click.mjs` | Launches BrowserOS neo headless on a fresh profile, serves one fixture page per case, connects playwright-core 1.64.1 with `connectOverCDP`, and records each click. |
| `click-goldens.json` | One object per case (`ok`, `strict`, `hidden`, `covered`, `disabled`), plus versions, launch args, and a cleanup report. |
| `selector-corpus.ts` | Mirrors Playwright's chain composition on top of Playwright's own `locatorUtils.ts` and `stringUtils.ts`. |
| `selector-goldens.json` | 94 entries of `{ chain, selector }`. `chain` is the JS an agent writes, `selector` is Playwright's selector string. |
| `selector-crosscheck.json` | Every chain re-evaluated against real playwright-core 1.64.1. The current result is 94/94 matching. |

## Rerun

```sh
cd /Users/shadowfax/llm/claw-sdk-v2/runs/neo-locators-click/goldens
npm ci                     # playwright-core 1.64.1, pinned in package.json
bun selector-corpus.ts     # writes selector-goldens.json
node capture-click.mjs     # writes click-goldens.json and selector-crosscheck.json (~13s)
```

Run the corpus first, because `capture-click.mjs` cross-checks whatever `selector-goldens.json` holds. The click capture runs under Node, not Bun, so that Playwright's `connectOverCDP` cannot hit Bun's WebSocket hang. `capture-click.mjs` exits non-zero when a selector mismatches or when any browser process survives cleanup. `selector-corpus.ts` refuses to run when the Playwright checkout is not at `26137face`.

## Versions

- playwright-core 1.64.1 for the click capture and the cross-check.
- Playwright checkout `/Users/shadowfax/code/oss/playwright` at `26137face` (1.66.0-next) for the corpus. `git diff v1.64.1 26137face` leaves `locatorUtils.ts` and `stringUtils.ts` unchanged and touches no composition method in `client/locator.ts`, `frame.ts` or `page.ts`. The 94/94 cross-check confirms it.
- BrowserOS neo 0.52.1.1, Chrome/155.0.8318.26, V8 15.5.35.14.
- Node v26.4.0 and Bun 1.3.6.

## click-goldens.json fields

Each case has these fields:

- `source`. The agent-side code.
- `fixtureBody` and `fixtureHtml`. The served page.
- `outcome`. Either `resolved` or `threw`.
- `resolvedType`, `url` and `urlRelative`. Set for `ok` only.
- `error`. Set for the other four cases. It holds `className` (`e.constructor.name`), `name`, `instanceOfTimeoutError`, `firstLine`, `message`, and `messageSplit` (`e.message.split('Call log:')`).
- `page`. Facts read from the page after the click: hash, active element, each button's box and the element at its center, and every pointer and mouse event the page saw.
- `cdp`. Playwright's own CDP traffic for the click in both directions. Strings are truncated to 240 characters.
- `elapsedMsPerRun` and `identicalAcrossRuns`. Each case runs three times on fresh pages. `page` and `cdp` come from the first run.

## Deviations and caveats

- **Colors are pinned off.** Playwright dims the call log with ANSI codes whenever the environment suggests color support (`COLORTERM`, `TERM=xterm-*`), even without a TTY. The script sets `FORCE_COLOR=0`, so the messages are plain text. With colors on, each call-log line is wrapped in `\x1b[2m` and `\x1b[22m` and nothing else changes.
- **`className` is a bundler artifact.** playwright-core 1.64.1 reports `TimeoutError2` for timeouts and `PlaywrightError` for the strict-mode error. Match on `name` (`TimeoutError` or `Error`) and `instanceOfTimeoutError` instead.
- **Fixtures add a recorder.** Each page carries a `<script>` in `<head>` that logs pointer and mouse events. The bodies are exactly as briefed. The overlay is `position:fixed`, covers the full viewport, and sits at `z-index:10`.
- **Retry call logs depend on timing.** The `covered` and `disabled` call logs follow Playwright's retry backoff (20, 100, 500 ms) against the timeout. They were identical within each of three script runs, and the last two script runs match each other exactly. A slower machine could still log one attempt fewer.
- **The corpus adds `description`.** It includes the `getByRole` `description` option, which the brief did not list. It exists in 1.64.1 and cross-checks.

## Observed behavior worth knowing

- `hidden`. `getByRole` skips `display:none` elements, so the locator never resolves. The call log holds only `waiting for getByRole(...)`, with no "not visible" line.
- `covered`. No input event reaches the page. Playwright scrolls, reads the content quads, and asks the injected script for the hit target, which returns `<div id="overlay"></div>`. Only then would it dispatch input, so it retries without sending any `Input.dispatchMouseEvent`.
- `ok`. Playwright sends `mouseMoved`, `mousePressed` (`force: 0.5`) and `mouseReleased` at the quad center (36.75, 18.5). `page.url()` already ends in `#done` when `click()` resolves.
- `strict`. It throws after about 20 ms, before any input. The message embeds `Error: ` after `locator.click: ` and lists each match with an `aka` locator (`.first()`, `.nth(1)`).
- `disabled`. Indentation in retry call logs is irregular. `waiting 20ms` sits shallower than `waiting 100ms`, and the last attempt logs its wait line once instead of `2 ×` and shifts its remaining lines one level shallower. Compare call logs byte for byte.
- Selectors. A `getByRole` name keeps raw tab and newline characters, while `getByText` JSON-escapes them. `hasText: ''` adds nothing. A RegExp ignores `exact`. Regexes with the `u` or `v` flag skip quote and `>>` escaping.
