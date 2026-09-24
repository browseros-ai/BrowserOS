# Expected fidelity checklist

One row per API form the corpus uses. Column two is real Playwright (1.4x, Chromium). Column
three is what a model would notice if the engine silently differed, which is what makes a
difference matter: a model debugs by reading the error and the returned value, so a wrong
value with `ok: true` is worse than a clear `Unsupported` error. The mediator grades both
engines against this table; "silent" means no error and no note in the result.

| API form | Real Playwright behaviour | If it silently differs, a model notices | Scripts |
|---|---|---|---|
| `const page = await context.newPage()` | Opens `about:blank` in the context; returns a `Page`. No URL argument. | Tab opens in the foreground and steals the user's focus; or the page is not in the session tab group (cockpit shows it as the user's). | all |
| `browser.newPage()` | Creates a fresh context and a page in it. | Works like `context.newPage()` here (one shared profile). A model notices only if cookies leak between "contexts" it expected isolated. | S09 |
| `page.goto(url)` | Waits for `load`; returns the response; throws on `net::ERR_*`; default 30 s. | Returns before `load` so the next locator misses; or does not throw on a bad URL, and the model reads an empty page. | all |
| `page.url()` | Synchronous string. | Returns a Promise: `page.url().endsWith` throws `endsWith is not a function`. | S01, S07 |
| `page.title()` | Async string of `document.title` at call time. | Stale title after a client-side change (S09 expects `Renamed`). | S02, S07, S09 |
| `getByLabel('Full name')` | Matches by `<label for>`, wrapping `<label>`, `aria-labelledby`, `aria-label`; substring, case-insensitive unless `exact`. | Wrapping-label checkbox (`I agree to the terms`) not found → `TimeoutError` on `check`. | S01 |
| `getByRole('button', { name })` | ARIA role + accessible name (accname); substring, case-insensitive unless `exact: true`; hidden elements excluded. | `name: 'new', exact: true` matching `Newest`; or a covered-but-visible button treated as hidden. | S01, S02, S04, S05, S07, S08, S09, F13 |
| `getByText('Loaded!')` | Text node substring match, whitespace-normalised, case-insensitive unless exact. | Matches the `<body>` (largest ancestor) instead of the innermost element; `innerText` then returns the whole page. | S06 |
| `getByPlaceholder('Search')` | `placeholder` attribute, substring. | Falls back to `aria-label` only → still works here; a model notices on inputs with a placeholder but no label. | S12 |
| `getByTestId('confirmation')` | `data-testid` exact match. | Substring match returns extra elements → strict-mode error on a page with `confirmation-2`. | S01, S06, S08, S12 |
| `locator('#items li')` | CSS, all matches; strict on actions. | Clicking a multi-match must throw `strict mode violation ... resolved to 3 elements`; a permissive engine clicks the first. | S02, S11, S12, F14 |
| `first()` / `nth(1)` | Index into the match list, zero-based. | Off-by-one: `nth(1)` returns `Item 1`. | S12 |
| `filter({ hasText })` | Narrows to elements containing the text (substring, case-insensitive). | Matches by exact text only, or matches ancestors too. | S12 |
| `frameLocator('#inner')` | Scopes later locators to the iframe's document; same rules inside. | Locator resolves in the outer document → `Frame button` not found. | S03 |
| `locator.fill(value)` | Waits actionable + editable, focuses, selects all, sets value, dispatches `input`/`change`. | No `input` event so React/Vue state does not update; no focus so a following `keyboard.press` goes elsewhere (S12). | S01, S12 |
| `selectOption('pro')` | Matches by value, then label; dispatches `change`. | Sets `.value` without `change` → framework state stale; label-only match fails on value. | S01 |
| `check()` | Waits actionable, clicks unless already checked, verifies the state. | Toggles an already-checked box, or sets `.checked` without a click event. | S01 |
| `click()` | Scrolls into view, waits visible + stable + enabled + receives events, trusted mouse events at the element centre. | Clicks through a covering banner (S08 `covered` is `null`), or uses `el.click()` so `mousedown`/`mouseup` handlers never fire. | many |
| `click({ timeout })` | Per-call override of the action timeout; failure throws `TimeoutError` named `TimeoutError`. | Ignores the option and waits the default; `error.name` is `Error` (S08 `covered`). | S08, F13 |
| `hover()` | Moves the mouse to the element centre; `mouseenter`/`mouseover` fire. | Dispatches synthetic events only → CSS `:hover` menus do not open; here the JS listener still fires, so only the count of visible items reveals it. | S08 |
| `page.keyboard.press('Enter')` | Key down/up on the focused element with a real `key`/`code`. | Goes to `<body>` because `fill` did not leave focus → `Searched:` never appears. | S12 |
| `locator.waitFor({ state, timeout })` | Polls until attached/visible; throws `TimeoutError`. | Returns immediately when the element is absent. | S06 |
| `page.waitForURL(glob \| regex)` | Waits until the URL matches (glob `**` supported) and `load` fires. | Resolves on the old URL because it already matched a loose pattern; items are read from the previous page. | S02 |
| `page.waitForLoadState()` | Waits for `load` (default) on the page. | Returns before the popup finished loading → title empty. | S07 |
| `expect(locator).toBeVisible()` | Auto-retries up to 5 s; fails with the locator, expected and received state. | No retry → fails at t=0 on the delayed element (S01 200 ms, S06 800 ms). | S01, S06 |
| `expect(locator).toHaveText(str)` | Retrying, whitespace-normalised full match; regex allowed. | Substring match passes a wrong value; no retry fails on late text. | S01, S03, S04, S08, S12 |
| `expect(locator).toHaveValue(str)` | Retrying, compares `input.value`. | Compares text content → always fails on inputs. | S12 |
| `expect(locator).toHaveCount(n)` | Retrying, exact count including 0. | Count of visible only, or 0 treated as "not found" error. | S06, S08 |
| `expect(page).toHaveURL(regex)` | Retrying on the page URL. | Compares against the initial URL only. | S02 |
| `expect(page).toHaveTitle(str)` | Retrying on `document.title`. | Reads the cached title → fails after a client-side rename. | S09 |
| `page.evaluate(fn, arg)` | Serialises the function, passes `arg` by value, returns JSON-serialisable result. | Function sent as `"[object Function]"` or `arg` dropped → `count` is `NaN` or 3, not 21. | S10 |
| `page.screenshot()` | Returns a `Buffer` (PNG). | Returns `{ path, base64 }` or a string: `Boolean(shot)` still true, so the corpus does not catch it; a model notices only when it tries `.length` or writes the file. | S10 |
| `page.on('dialog', handler)` | Handler receives `Dialog` with `type()`, `message()`, `accept()`; without a handler dialogs are auto-dismissed. | Click hangs until the run cap; or the dialog is accepted silently and `seen` is empty. | S04 |
| `waitForEvent('download')` + `Promise.all` | Resolves with a `Download`; `suggestedFilename()`, `path()` after the file is saved. | Navigates to the file instead (page shows text) or `path()` is `null`. | S05 |
| `context.pages()` | Synchronous array of the context's pages, including popups. | Popup missing (opened tab not tracked) → loop exhausts and `opened` is `undefined`. | S07 |
| `Promise.all` over pages | Independent pages proceed concurrently. | Serialised, so 3 gotos take 3× the time; or one shared cursor makes `locator('h1')` on page A read page B. | S11 |
| `page.close()` | Closes the tab; later calls throw `Target closed`. | Tab stays open (page count 1 not 0). | S09 |
| `textContent()` / `innerText()` / `allInnerTexts()` | `textContent` raw, `innerText` rendered, `allInnerTexts` per match. | `innerText` returns raw text with hidden nodes, or `allInnerTexts` returns one joined string. | many |
| `count()` | Number of matches right now, no waiting. | Waits for at least one → S02 never terminates on page 3. | S02, S11 |
| Errors: `TimeoutError` | `error.name === 'TimeoutError'`; message `Timeout 2000ms exceeded.` + call log naming the locator. | Generic message without the locator → a model cannot tell a bad selector from a slow page. | S08, F13 |
| Errors: strict mode | `Error: strict mode violation: locator('#items li') resolved to 3 elements:` + the elements. | Permissive click. | F14 |
| Errors: syntax | The script never runs; the tool reports a `SyntaxError` with line/column text. | Reported as a runtime error after a page was opened (a stray tab in the group). | F15 |
| Auto-wait defaults | Actions 30 s, `expect` 5 s, navigation 30 s. | This tool shortens actions to 5 s and navigation to 15 s under a 30 s call cap; a model notices only when a slow site times out sooner than Playwright would. Documented in the tool description. | all |
