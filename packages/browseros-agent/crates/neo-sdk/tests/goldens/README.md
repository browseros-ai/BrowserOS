# Playwright selector goldens for Neo

Recorded truth for Neo's locator builder: the selector string Playwright builds for each locator chain an agent writes. neo-runtime's `tests/selector_goldens.rs` builds every chain inside QuickJS and compares the strings byte for byte. These must match exactly, because the page's injected script parses whatever string Neo sends.

Clicks are not recorded here. They are tested by outcome (what the script saw, what the page saw, the delivery record) on the pages in `../pages/`: `neo-runtime/tests/click_e2e.rs` on Neo alone, and `../parity/click.test.mjs` beside live Playwright.

## Files

| File | Contents |
| --- | --- |
| `selector-corpus.ts` | Mirrors Playwright's chain composition on top of Playwright's own `locatorUtils.ts` and `stringUtils.ts`. |
| `selector-goldens.json` | 94 entries of `{ chain, selector }`. `chain` is the JS an agent writes, `selector` is Playwright's selector string. |

## Rerun

```sh
bun selector-corpus.ts     # writes selector-goldens.json
```

`selector-corpus.ts` refuses to run when the Playwright checkout is not at `26137face`. When the corpus was captured, every chain was also evaluated against real playwright-core 1.64.1, and all 94 matched.

## Versions

- Playwright checkout at `26137face` (1.66.0-next) for the corpus. `git diff v1.64.1 26137face` leaves `locatorUtils.ts` and `stringUtils.ts` unchanged and touches no composition method in `client/locator.ts`, `frame.ts` or `page.ts`. The 94/94 cross-check confirms it.
- playwright-core 1.64.1 for the cross-check.

## Worth knowing

- **`className` is a bundler artifact.** playwright-core 1.64.1 reports `TimeoutError2` for timeouts and `PlaywrightError` for a strict mode violation. Scripts and tests compare `e.name` (`TimeoutError` or `Error`), never `e.constructor.name`.
- **The corpus adds `description`.** It includes the `getByRole` `description` option, which the brief did not list. It exists in 1.64.1 and cross-checks.
- **Selectors.** A `getByRole` name keeps raw tab and newline characters, while `getByText` JSON-escapes them. `hasText: ''` adds nothing. A RegExp ignores `exact`. Regexes with the `u` or `v` flag skip quote and `>>` escaping.
