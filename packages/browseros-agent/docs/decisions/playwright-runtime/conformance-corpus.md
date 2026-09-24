# Playwright conformance corpus

Engine-independent acceptance tests for the Playwright tool. Fifteen scripts, exactly as an
agent would send them as the tool's `code` argument (top-level `await`, `return`). Twelve
must pass, three must fail with a named error class. Fixtures live in `fixtures/` and are
known-good (`fixtures/KNOWN-GOOD.md`, checked with raw CDP on an isolated BrowserOS neo).

## Conventions

- `BASE` is the fixture server origin printed by `bun fixtures/serve.ts` (for example
  `http://127.0.0.1:63388`). `run-corpus.ts` replaces the literal text `BASE/` in the script
  and in the expected value.
- **Tool name**: written as `playwright` here. Substitute the mediated name; nothing else changes.
- **One fresh MCP session per script**, so "pages in the session tab group afterwards" is
  per script, counted before the session is closed.
- **Child audit rows** are the `toolName` values of the dispatch rows whose `parentDispatchId`
  equals the script row's `dispatchKey`, in `createdAt` order. Names are Playwright method
  names. Notation in `rows`: `name` exactly once; `name+` one or more consecutive;
  `name*` zero or more; `[name]` optional; `{a, b}` those rows in any order. Accepted
  aliases (an engine may record the right-hand form): `context.newPage` = `pages.newPage`,
  `context.pages` = `pages.list`, `page.close` = `pages.close`, `keyboard.press` =
  `page.keyboard.press`, `expect.X` = `assert.X`. `page.waitForTimeout` rows are ignored.
- **Error classes** are matched on the envelope's `error` string: `TimeoutError` →
  `/^TimeoutError\b|TimeoutError:/`; `Error` (strict mode) → `/strict mode violation/`;
  `SyntaxError` → `/SyntaxError|syntax error/i`.
- `errorRows` lists zero-based indexes of child rows whose `resultMeta.isError` must be true.
- Each entry has a `json` block the runner reads; the prose above it is for humans.

---

### S01 form: labelled inputs, password, select, checkbox, submit, confirmation

Fixtures: `BASE/form.html`. Covers `context.newPage`, `page.goto`, `getByLabel`, `fill`,
`selectOption`, `check`, `getByRole` + `click`, `getByTestId`, `expect.toBeVisible`,
`expect.toHaveText`, `textContent`, `page.url()`. The confirmation renders 200 ms after
submit, so `toBeVisible` must auto-wait.

```js
const page = await context.newPage();
await page.goto('BASE/form.html');
await page.getByLabel('Full name').fill('Ada Lovelace');
await page.getByLabel('Email').fill('ada@example.com');
await page.getByLabel('Password').fill('s3cret!');
await page.getByLabel('Plan').selectOption('pro');
await page.getByLabel('I agree to the terms').check();
await page.getByRole('button', { name: 'Create account' }).click();
const confirmation = page.getByTestId('confirmation');
await expect(confirmation).toBeVisible();
await expect(confirmation).toHaveText('Account created for Ada Lovelace (pro, terms: true)');
return { confirmation: await confirmation.textContent(), url: page.url() };
```

```json
{ "id": "S01", "ok": true,
  "value": { "confirmation": "Account created for Ada Lovelace (pro, terms: true)", "url": "BASE/form.html" },
  "rows": ["context.newPage", "page.goto", "locator.fill", "locator.fill", "locator.fill", "locator.selectOption",
           "locator.check", "locator.click", "expect.toBeVisible", "expect.toHaveText", "locator.textContent"],
  "pages": 1 }
```

### S02 pagination loop over three pages

Fixtures: `BASE/list-1.html`, `list-2.html`, `list-3.html`. Covers `locator(css)`,
`allInnerTexts`, `getByRole('link')`, `count`, `click`, `waitForURL(glob)`,
`expect.toHaveURL(regex)`, `page.title()`. The `Next` link is absent on page 3.

```js
const page = await context.newPage();
await page.goto('BASE/list-1.html');
const items = [];
for (let i = 0; i < 10; i++) {
  items.push(...(await page.locator('#items li').allInnerTexts()));
  const next = page.getByRole('link', { name: 'Next' });
  if ((await next.count()) === 0) break;
  await next.click();
  await page.waitForURL('**/list-' + (i + 2) + '.html');
}
await expect(page).toHaveURL(/list-3\.html$/);
return { items, last: await page.title() };
```

```json
{ "id": "S02", "ok": true,
  "value": { "items": ["Item 1", "Item 2", "Item 3", "Item 4", "Item 5", "Item 6", "Item 7", "Item 8", "Item 9"],
             "last": "Corpus list page 3" },
  "rows": ["context.newPage", "page.goto",
           "locator.allInnerTexts", "locator.count", "locator.click", "page.waitForURL",
           "locator.allInnerTexts", "locator.count", "locator.click", "page.waitForURL",
           "locator.allInnerTexts", "locator.count", "expect.toHaveURL", "page.title"],
  "pages": 1 }
```

### S03 same-origin iframe through frameLocator

Fixtures: `BASE/frame.html` (+ `frame-inner.html`). Covers `frameLocator`, `getByRole`
inside a frame, `click`, `expect.toHaveText`, `textContent` in both frames.

```js
const page = await context.newPage();
await page.goto('BASE/frame.html');
const frame = page.frameLocator('#inner');
await frame.getByRole('button', { name: 'Frame button' }).click();
await expect(frame.locator('#frame-status')).toHaveText('frame clicked');
return {
  outer: await page.locator('#outer-status').textContent(),
  inner: await frame.locator('#frame-status').textContent(),
};
```

```json
{ "id": "S03", "ok": true,
  "value": { "outer": "outer idle", "inner": "frame clicked" },
  "rows": ["context.newPage", "page.goto", "locator.click", "expect.toHaveText", "locator.textContent", "locator.textContent"],
  "pages": 1 }
```

### S04 confirm() dialog handled by page.on('dialog')

Fixtures: `BASE/dialog.html`. Covers `page.on('dialog')`, `dialog.type()`, `dialog.message()`,
`dialog.accept()`, a click that blocks on the dialog, `expect.toHaveText`.

```js
const page = await context.newPage();
await page.goto('BASE/dialog.html');
const seen = [];
page.on('dialog', async (dialog) => {
  seen.push({ type: dialog.type(), message: dialog.message() });
  await dialog.accept();
});
await page.getByRole('button', { name: 'Delete' }).click();
await expect(page.locator('#status')).toHaveText('Deleted');
return { seen, status: await page.locator('#status').textContent() };
```

```json
{ "id": "S04", "ok": true,
  "value": { "seen": [{ "type": "confirm", "message": "Delete it?" }], "status": "Deleted" },
  "rows": ["context.newPage", "page.goto", "locator.click", "[dialog.accept]", "expect.toHaveText", "locator.textContent"],
  "pages": 1 }
```

### S05 download via waitForEvent('download') + Promise.all

Fixtures: `BASE/download.html` (+ `notes.txt`, served as an attachment). Covers the canonical
Playwright download idiom, `download.suggestedFilename()`, `download.path()`.

```js
const page = await context.newPage();
await page.goto('BASE/download.html');
const [download] = await Promise.all([
  page.waitForEvent('download'),
  page.getByRole('link', { name: 'Download notes' }).click(),
]);
const path = await download.path();
return { name: download.suggestedFilename(), saved: typeof path === 'string' && path.length > 0 };
```

```json
{ "id": "S05", "ok": true,
  "value": { "name": "notes.txt", "saved": true },
  "rows": ["context.newPage", "page.goto", "{page.waitForEvent, locator.click}", "[download.path]"],
  "pages": 1 }
```

### S06 element that appears after 800 ms

Fixtures: `BASE/delayed.html`. Covers `getByText`, `locator.waitFor({ state, timeout })`,
`expect.toBeVisible`, `expect.toHaveCount(0)`, `innerText`.

```js
const page = await context.newPage();
await page.goto('BASE/delayed.html');
await page.getByText('Loaded!').waitFor({ state: 'visible', timeout: 5000 });
await expect(page.getByTestId('loaded')).toBeVisible();
await expect(page.locator('#status')).toHaveCount(0);
return { text: await page.getByTestId('loaded').innerText() };
```

```json
{ "id": "S06", "ok": true,
  "value": { "text": "Loaded!" },
  "rows": ["context.newPage", "page.goto", "locator.waitFor", "expect.toBeVisible", "expect.toHaveCount", "locator.innerText"],
  "pages": 1 }
```

### S07 target=_blank link, then context.pages()

Fixtures: `BASE/newtab.html` (+ `opened.html`). Covers `context.pages()` after a `_blank`
click, `page.waitForLoadState`, `page.title`, `page.url()`. Written with `await context.pages()`,
which is valid in real Playwright (awaiting an array) and lets an engine make it a bridged
call. The popup must land in the session's tab group: `pages` is 2.

```js
const page = await context.newPage();
await page.goto('BASE/newtab.html');
const before = (await context.pages()).length;
await page.getByRole('link', { name: 'Open in new tab' }).click();
let opened;
for (let i = 0; i < 20 && !opened; i++) {
  opened = (await context.pages()).find((p) => p.url().endsWith('/opened.html'));
  if (!opened) await page.waitForTimeout(250);
}
await opened.waitForLoadState();
return {
  title: await opened.title(),
  h1: await opened.locator('h1').textContent(),
  added: (await context.pages()).length - before,
};
```

```json
{ "id": "S07", "ok": true,
  "value": { "title": "Corpus opened", "h1": "Opened", "added": 1 },
  "rows": ["context.newPage", "page.goto", "context.pages*", "locator.click", "context.pages*",
           "page.waitForLoadState", "page.title", "locator.textContent", "context.pages*"],
  "pages": 2 }
```

### S08 covered element (consent banner), then hover menu

Fixtures: `BASE/banner.html`. Covers actionability (a click on a covered element must time
out with `TimeoutError`, not click through), `click`, `hover`, `expect.toHaveCount`,
`allInnerTexts`, `error.name`.

```js
const page = await context.newPage();
await page.goto('BASE/banner.html');
let covered = null;
try {
  await page.getByRole('button', { name: 'Buy' }).click({ timeout: 1500 });
} catch (error) {
  covered = error.name;
}
await page.getByRole('button', { name: 'Accept' }).click();
await page.getByRole('button', { name: 'Buy' }).click();
await expect(page.getByTestId('buy-status')).toHaveText('Bought');
await page.getByTestId('menu-trigger').hover();
await expect(page.getByRole('menuitem')).toHaveCount(3);
return {
  covered,
  status: await page.getByTestId('buy-status').textContent(),
  menu: await page.getByRole('menuitem').allInnerTexts(),
};
```

```json
{ "id": "S08", "ok": true,
  "value": { "covered": "TimeoutError", "status": "Bought", "menu": ["Orders", "Returns", "Account"] },
  "rows": ["context.newPage", "page.goto", "locator.click", "locator.click", "locator.click", "expect.toHaveText",
           "locator.hover", "expect.toHaveCount", "locator.textContent", "locator.allInnerTexts"],
  "errorRows": [2],
  "pages": 1 }
```

### S09 title change, browser.newPage form, page.close

Fixtures: `BASE/title.html`. Covers `browser.newPage()`, `page.title()`, `click`,
`expect.toHaveTitle`, `page.close()`. The page is closed, so `pages` is 0.

```js
const page = await browser.newPage();
await page.goto('BASE/title.html');
const before = await page.title();
await page.getByRole('button', { name: 'Rename' }).click();
await expect(page).toHaveTitle('Renamed');
const after = await page.title();
await page.close();
return { before, after };
```

```json
{ "id": "S09", "ok": true,
  "value": { "before": "Corpus title", "after": "Renamed" },
  "rows": ["context.newPage", "page.goto", "page.title", "locator.click", "expect.toHaveTitle", "page.title", "page.close"],
  "pages": 0 }
```

### S10 page.evaluate with a function and an argument, page.screenshot

Fixtures: `BASE/list-2.html`. Covers `page.evaluate(fn, arg)` (a real function, not a
string), `page.evaluate(fn)`, `page.screenshot()` returning something truthy.

```js
const page = await context.newPage();
await page.goto('BASE/list-2.html');
const count = await page.evaluate((factor) => document.querySelectorAll('#items li').length * factor, 7);
const heading = await page.evaluate(() => document.querySelector('h1').textContent);
const shot = await page.screenshot({ type: 'png' });
return { count, heading, screenshot: Boolean(shot) };
```

```json
{ "id": "S10", "ok": true,
  "value": { "count": 21, "heading": "Page 2", "screenshot": true },
  "rows": ["context.newPage", "page.goto", "page.evaluate", "page.evaluate", "page.screenshot"],
  "pages": 1 }
```

### S11 Promise.all over three pages

Fixtures: `BASE/list-1.html`, `list-2.html`, `list-3.html`. Covers concurrent `newPage`,
`goto`, `textContent`, `count` across pages. Row order inside each `{}` group is free.

```js
const urls = ['BASE/list-1.html', 'BASE/list-2.html', 'BASE/list-3.html'];
const pages = await Promise.all(urls.map(() => context.newPage()));
await Promise.all(pages.map((p, i) => p.goto(urls[i])));
const headings = await Promise.all(pages.map((p) => p.locator('h1').textContent()));
const counts = await Promise.all(pages.map((p) => p.locator('#items li').count()));
return { headings, counts };
```

```json
{ "id": "S11", "ok": true,
  "value": { "headings": ["Page 1", "Page 2", "Page 3"], "counts": [3, 3, 3] },
  "rows": ["{context.newPage, context.newPage, context.newPage}", "{page.goto, page.goto, page.goto}",
           "{locator.textContent, locator.textContent, locator.textContent}", "{locator.count, locator.count, locator.count}"],
  "pages": 3 }
```

### S12 placeholder, toHaveValue, keyboard.press, first/nth/filter

Fixtures: `BASE/form.html`, `BASE/list-1.html`. Covers `getByPlaceholder`, `fill`,
`expect.toHaveValue`, `page.keyboard.press('Enter')` on the focused input, a second
`goto` on the same page, `nth(1)`, `first()`, `filter({ hasText })`.

```js
const page = await context.newPage();
await page.goto('BASE/form.html');
const search = page.getByPlaceholder('Search');
await search.fill('lovelace');
await expect(search).toHaveValue('lovelace');
await page.keyboard.press('Enter');
await expect(page.getByTestId('search-result')).toHaveText('Searched: lovelace');
await page.goto('BASE/list-1.html');
const second = await page.locator('#items li').nth(1).textContent();
const first = await page.locator('#items li').first().textContent();
const filtered = await page.locator('#items li').filter({ hasText: 'Item 3' }).textContent();
return { second, first, filtered };
```

```json
{ "id": "S12", "ok": true,
  "value": { "second": "Item 2", "first": "Item 1", "filtered": "Item 3" },
  "rows": ["context.newPage", "page.goto", "locator.fill", "expect.toHaveValue", "keyboard.press", "expect.toHaveText",
           "page.goto", "locator.textContent", "locator.textContent", "locator.textContent"],
  "pages": 1 }
```

### F13 must fail: locator that matches nothing, `{ timeout: 2000 }`

Fixtures: `BASE/form.html`. Expected error class `TimeoutError`. The message must name the
timeout and the locator, in Playwright's words: `Timeout 2000ms exceeded` and
`getByRole('button', { name: 'Launch rocket' })`. The click row is an error row with
`durationMs` close to 2000.

```js
const page = await context.newPage();
await page.goto('BASE/form.html');
await page.getByRole('button', { name: 'Launch rocket' }).click({ timeout: 2000 });
return 'unreachable';
```

```json
{ "id": "F13", "ok": false, "errorClass": "TimeoutError",
  "errorContains": ["Timeout 2000ms exceeded", "Launch rocket"],
  "rows": ["context.newPage", "page.goto", "locator.click"],
  "errorRows": [2],
  "pages": 1 }
```

### F14 must fail: strict-mode violation

Fixtures: `BASE/list-1.html`. Expected error class `Error` whose message contains
`strict mode violation` and `resolved to 3 elements`. A permissive engine that clicks the
first `li` fails this test.

```js
const page = await context.newPage();
await page.goto('BASE/list-1.html');
await page.locator('#items li').click();
return 'unreachable';
```

```json
{ "id": "F14", "ok": false, "errorClass": "Error",
  "errorContains": ["strict mode violation", "resolved to 3 elements"],
  "rows": ["context.newPage", "page.goto", "locator.click"],
  "errorRows": [2],
  "pages": 1 }
```

### F15 must fail: syntax error

Fixtures: none. Expected error class `SyntaxError`, reported before anything runs: no child
rows, no pages. The parent dispatch row has `isError: true`.

```js
const page = await context.newPage(;
await page.goto('BASE/form.html');
return 'unreachable';
```

```json
{ "id": "F15", "ok": false, "errorClass": "SyntaxError",
  "rows": [],
  "pages": 0 }
```

---

## Decisions

DECIDED: one fresh MCP session per script.
WHY: makes the tab-group page count and the child-row list unambiguous; the runner does it.
REVISIT: add a multi-call script (carry a URL across two calls) once the tool exists.

DECIDED: `context.pages()` is written as `await context.pages()`.
WHY: valid in real Playwright and lets an engine implement it as a bridged call; a bare
sync call is the fidelity item recorded in `expected-fidelity.md`.
REVISIT: never.

DECIDED: a popup opened by an owned tab (S07) counts as a session page.
WHY: the tab-group-per-session invariant says pages a script opens join the group; a `_blank`
click opens one. Today's `run` hook only claims `pages.newPage`, so this is new host work
whichever engine wins.
REVISIT: if the host cannot observe `Target.targetCreated.openerId`, drop `pages` to 1 for S07
and keep the value check.

DECIDED: S08 asserts the covered click fails with `TimeoutError` before the banner is dismissed.
WHY: actionability is the behaviour agents rely on most; an engine that clicks through the
banner passes the naive form and fails users.
REVISIT: never.

ASSUMPTION: `resultMeta` on a child row is the JSON `result_meta` string with `isError` (as
`script_hook.rs::child_result_meta` writes it today).
ASSUMPTION: the download in S05 works in a headless isolated browser once the engine sets
`Browser.setDownloadBehavior`; the fixture serves `Content-Disposition: attachment`.
ASSUMPTION: `page.screenshot()` returns a truthy value in every engine (Buffer or an object);
the corpus only asserts truthiness.
