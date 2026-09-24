/**
 * The twenty-five Playwright corpus scripts exercise the real MCP tool, then its
 * persisted child dispatches and live session page projection. Each script
 * owns a fresh session and fixture server; teardown also removes unclaimed
 * popups so an ownership regression cannot contaminate later suite cases.
 */
import assert from 'node:assert/strict'
import { resolve } from 'node:path'
import type { CaseContext, ContractCase } from './cases'
import { apiGet, expectOk, waitUntil } from './helpers'
import { type McpSession, type McpToolResult, textOf } from './mcp-client'

interface CorpusScript {
  id: string
  title: string
  code: string
  ok: boolean
  value?: unknown
  errorClass?: 'TimeoutError' | 'Error' | 'SyntaxError'
  errorContains?: string[]
  rows: string[]
  errorRows?: number[]
  pages: number
}

// S01–F15 are ported from playwright-runtime/conformance/corpus.md. S16–S25
// cover common pasted agent scripts. BASE/ and UPLOAD_FILE are harness inputs;
// the deliberately invalid F15 must stay invalid.
const corpus: CorpusScript[] = [
  {
    id: 'S01',
    ok: true,
    value: {
      confirmation: 'Account created for Ada Lovelace (pro, terms: true)',
      url: 'BASE/form.html',
    },
    rows: [
      'context.newPage',
      'page.goto',
      'locator.fill',
      'locator.fill',
      'locator.fill',
      'locator.selectOption',
      'locator.check',
      'locator.click',
      'expect.toBeVisible',
      'expect.toHaveText',
      'locator.textContent',
    ],
    pages: 1,
    title:
      'S01 form: labelled inputs, password, select, checkbox, submit, confirmation',
    code: `const page = await context.newPage();
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
return { confirmation: await confirmation.textContent(), url: page.url() };`,
  },
  {
    id: 'S02',
    ok: true,
    value: {
      items: [
        'Item 1',
        'Item 2',
        'Item 3',
        'Item 4',
        'Item 5',
        'Item 6',
        'Item 7',
        'Item 8',
        'Item 9',
      ],
      last: 'Corpus list page 3',
    },
    rows: [
      'context.newPage',
      'page.goto',
      'locator.allInnerTexts',
      'locator.count',
      'locator.click',
      'page.waitForURL',
      'locator.allInnerTexts',
      'locator.count',
      'locator.click',
      'page.waitForURL',
      'locator.allInnerTexts',
      'locator.count',
      'expect.toHaveURL',
      'page.title',
    ],
    pages: 1,
    title: 'S02 pagination loop over three pages',
    code: String.raw`const page = await context.newPage();
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
return { items, last: await page.title() };`,
  },
  {
    id: 'S03',
    ok: true,
    value: {
      outer: 'outer idle',
      inner: 'frame clicked',
    },
    rows: [
      'context.newPage',
      'page.goto',
      'locator.click',
      'expect.toHaveText',
      'locator.textContent',
      'locator.textContent',
    ],
    pages: 1,
    title: 'S03 same-origin iframe through frameLocator',
    code: `const page = await context.newPage();
await page.goto('BASE/frame.html');
const frame = page.frameLocator('#inner');
await frame.getByRole('button', { name: 'Frame button' }).click();
await expect(frame.locator('#frame-status')).toHaveText('frame clicked');
return {
  outer: await page.locator('#outer-status').textContent(),
  inner: await frame.locator('#frame-status').textContent(),
};`,
  },
  {
    id: 'S04',
    ok: true,
    value: {
      seen: [
        {
          type: 'confirm',
          message: 'Delete it?',
        },
      ],
      status: 'Deleted',
    },
    rows: [
      'context.newPage',
      'page.goto',
      // The armed event wait and its dialog handler can finish before the click
      // they unblock. Audit rows record completion, not JavaScript call order.
      '{page.waitForEvent, page.dialog, locator.click}',
      'expect.toHaveText',
      'locator.textContent',
    ],
    pages: 1,
    title: "S04 confirm() dialog handled by page.on('dialog')",
    code: `const page = await context.newPage();
await page.goto('BASE/dialog.html');
const seen = [];
page.on('dialog', async (dialog) => {
  seen.push({ type: dialog.type(), message: dialog.message() });
  await dialog.accept();
});
await page.getByRole('button', { name: 'Delete' }).click();
await expect(page.locator('#status')).toHaveText('Deleted');
return { seen, status: await page.locator('#status').textContent() };`,
  },
  {
    id: 'S05',
    ok: true,
    value: {
      name: 'notes.txt',
      saved: true,
    },
    rows: [
      'context.newPage',
      'page.goto',
      '{page.waitForEvent, locator.click}',
      '[download.path]',
    ],
    pages: 1,
    title: "S05 download via waitForEvent('download') + Promise.all",
    code: `const page = await context.newPage();
await page.goto('BASE/download.html');
const [download] = await Promise.all([
  page.waitForEvent('download'),
  page.getByRole('link', { name: 'Download notes' }).click(),
]);
const path = await download.path();
return { name: download.suggestedFilename(), saved: typeof path === 'string' && path.length > 0 };`,
  },
  {
    id: 'S06',
    ok: true,
    value: {
      text: 'Loaded!',
    },
    rows: [
      'context.newPage',
      'page.goto',
      'locator.waitFor',
      'expect.toBeVisible',
      'expect.toHaveCount',
      'locator.innerText',
    ],
    pages: 1,
    title: 'S06 element that appears after 800 ms',
    code: `const page = await context.newPage();
await page.goto('BASE/delayed.html');
await page.getByText('Loaded!').waitFor({ state: 'visible', timeout: 5000 });
await expect(page.getByTestId('loaded')).toBeVisible();
await expect(page.locator('#status')).toHaveCount(0);
return { text: await page.getByTestId('loaded').innerText() };`,
  },
  {
    id: 'S07',
    ok: true,
    value: {
      title: 'Corpus opened',
      h1: 'Opened',
      added: 1,
    },
    rows: [
      'context.newPage',
      'page.goto',
      'context.pages*',
      'locator.click',
      'context.pages*',
      'page.waitForLoadState',
      'page.title',
      'locator.textContent',
      'context.pages*',
    ],
    pages: 2,
    title: 'S07 target=_blank link, then context.pages()',
    code: `const page = await context.newPage();
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
};`,
  },
  {
    id: 'S08',
    ok: true,
    value: {
      covered: 'TimeoutError',
      status: 'Bought',
      menu: ['Orders', 'Returns', 'Account'],
    },
    rows: [
      'context.newPage',
      'page.goto',
      'locator.click',
      'locator.click',
      'locator.click',
      'expect.toHaveText',
      'locator.hover',
      'expect.toHaveCount',
      'locator.textContent',
      'locator.allInnerTexts',
    ],
    errorRows: [2],
    pages: 1,
    title: 'S08 covered element (consent banner), then hover menu',
    code: `const page = await context.newPage();
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
};`,
  },
  {
    id: 'S09',
    ok: true,
    value: {
      before: 'Corpus title',
      after: 'Renamed',
    },
    rows: [
      'context.newPage',
      'page.goto',
      'page.title',
      'locator.click',
      'expect.toHaveTitle',
      'page.title',
      'page.close',
    ],
    pages: 0,
    title: 'S09 title change, browser.newPage form, page.close',
    code: `const page = await browser.newPage();
await page.goto('BASE/title.html');
const before = await page.title();
await page.getByRole('button', { name: 'Rename' }).click();
await expect(page).toHaveTitle('Renamed');
const after = await page.title();
await page.close();
return { before, after };`,
  },
  {
    id: 'S10',
    ok: true,
    value: {
      count: 21,
      heading: 'Page 2',
      screenshot: true,
    },
    rows: [
      'context.newPage',
      'page.goto',
      'page.evaluate',
      'page.evaluate',
      'page.screenshot',
    ],
    pages: 1,
    title: 'S10 page.evaluate with a function and an argument, page.screenshot',
    code: `const page = await context.newPage();
await page.goto('BASE/list-2.html');
const count = await page.evaluate((factor) => document.querySelectorAll('#items li').length * factor, 7);
const heading = await page.evaluate(() => document.querySelector('h1').textContent);
const shot = await page.screenshot({ type: 'png' });
return { count, heading, screenshot: Boolean(shot) };`,
  },
  {
    id: 'S11',
    ok: true,
    value: {
      headings: ['Page 1', 'Page 2', 'Page 3'],
      counts: [3, 3, 3],
    },
    rows: [
      '{context.newPage, context.newPage, context.newPage}',
      '{page.goto, page.goto, page.goto}',
      '{locator.textContent, locator.textContent, locator.textContent}',
      '{locator.count, locator.count, locator.count}',
    ],
    pages: 3,
    title: 'S11 Promise.all over three pages',
    code: `const urls = ['BASE/list-1.html', 'BASE/list-2.html', 'BASE/list-3.html'];
const pages = await Promise.all(urls.map(() => context.newPage()));
await Promise.all(pages.map((p, i) => p.goto(urls[i])));
const headings = await Promise.all(pages.map((p) => p.locator('h1').textContent()));
const counts = await Promise.all(pages.map((p) => p.locator('#items li').count()));
return { headings, counts };`,
  },
  {
    id: 'S12',
    ok: true,
    value: {
      second: 'Item 2',
      first: 'Item 1',
      filtered: 'Item 3',
    },
    rows: [
      'context.newPage',
      'page.goto',
      'locator.fill',
      'expect.toHaveValue',
      'keyboard.press',
      'expect.toHaveText',
      'page.goto',
      'locator.textContent',
      'locator.textContent',
      'locator.textContent',
    ],
    pages: 1,
    title: 'S12 placeholder, toHaveValue, keyboard.press, first/nth/filter',
    code: `const page = await context.newPage();
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
return { second, first, filtered };`,
  },
  {
    id: 'F13',
    ok: false,
    errorClass: 'TimeoutError',
    errorContains: ['Timeout 2000ms exceeded', 'Launch rocket'],
    rows: ['context.newPage', 'page.goto', 'locator.click'],
    errorRows: [2],
    pages: 1,
    title: 'F13 must fail: locator that matches nothing, `{ timeout: 2000 }`',
    code: `const page = await context.newPage();
await page.goto('BASE/form.html');
await page.getByRole('button', { name: 'Launch rocket' }).click({ timeout: 2000 });
return 'unreachable';`,
  },
  {
    id: 'F14',
    ok: false,
    errorClass: 'Error',
    errorContains: ['strict mode violation', 'resolved to 3 elements'],
    rows: ['context.newPage', 'page.goto', 'locator.click'],
    errorRows: [2],
    pages: 1,
    title: 'F14 must fail: strict-mode violation',
    code: `const page = await context.newPage();
await page.goto('BASE/list-1.html');
await page.locator('#items li').click();
return 'unreachable';`,
  },
  {
    id: 'F15',
    ok: false,
    errorClass: 'SyntaxError',
    rows: [],
    pages: 0,
    title: 'F15 must fail: syntax error',
    code: `const page = await context.newPage(;
await page.goto('BASE/form.html');
return 'unreachable';`,
  },
  {
    id: 'S16',
    ok: true,
    pages: 1,
    title: 'S16 shadow roots, regex and exact roles, aria-labelledby',
    value: { saved: 'saved', exact: 1, regex: 2, name: 'Ada' },
    rows: [
      'context.newPage',
      'page.goto',
      'locator.click',
      'locator.fill',
      'locator.textContent',
      'locator.count',
      'locator.count',
      'locator.inputValue',
    ],
    code: `const page = await context.newPage();
await page.goto('BASE/hardening.html');
await page.locator('#shadow-host').getByRole('button', { name: /save profile/i }).click();
await page.getByLabel('Display name').fill('Ada');
return { saved: await page.locator('#shadow-host output').textContent(), exact: await page.getByRole('button', { name: 'Save', exact: true }).count(), regex: await page.getByRole('button', { name: /^Save(?: draft)?$/ }).count(), name: await page.getByLabel('Display name').inputValue() };`,
  },
  {
    id: 'S17',
    ok: true,
    pages: 1,
    title: 'S17 multiple select and negated attribute, class, count assertions',
    value: ['rust', 'css'],
    rows: [
      'context.newPage',
      'page.goto',
      'locator.selectOption',
      'expect.toHaveAttribute',
      'expect.toHaveAttribute',
      'expect.toHaveClass',
      'expect.toHaveClass',
      'expect.toHaveCount',
      'expect.toHaveCount',
      'locator.evaluate',
    ],
    code: `const page = await context.newPage();
await page.goto('BASE/hardening.html');
const topics = page.getByLabel('Topics');
await topics.selectOption(['rust', 'css']);
await expect(topics).toHaveAttribute('data-state', 'ready');
await expect(topics).not.toHaveAttribute('data-state', 'busy');
await expect(topics).toHaveClass('ready');
await expect(topics).not.toHaveClass('busy');
await expect(page.locator('#items li')).toHaveCount(3);
await expect(page.locator('#items li')).not.toHaveCount(2);
return await topics.evaluate(el => [...el.selectedOptions].map(option => option.value));`,
  },
  {
    id: 'S18',
    ok: true,
    pages: 1,
    title: 'S18 setInputFiles uploads a real temporary file',
    value: { name: 'agent-upload.txt', size: 6 },
    rows: [
      'context.newPage',
      'page.goto',
      'locator.setInputFiles',
      'locator.evaluate',
    ],
    code: `const page = await context.newPage();
await page.goto('BASE/hardening.html');
await page.getByLabel('Attachment').setInputFiles(UPLOAD_FILE);
return await page.getByLabel('Attachment').evaluate(el => ({ name: el.files[0].name, size: el.files[0].size }));`,
  },
  {
    id: 'S19',
    ok: true,
    pages: 1,
    title: 'S19 prompt accepts text from a once dialog handler',
    value: 'Hello Ada',
    rows: [
      'context.newPage',
      'page.goto',
      '{page.waitForEvent, page.dialog, locator.click}',
      'expect.toHaveText',
      'locator.textContent',
    ],
    code: `const page = await context.newPage();
await page.goto('BASE/hardening.html');
page.once('dialog', dialog => dialog.accept('Ada'));
await page.getByRole('button', { name: 'Ask name' }).click();
await expect(page.locator('#prompt-result')).toHaveText('Hello Ada');
return await page.locator('#prompt-result').textContent();`,
  },
  {
    id: 'S20',
    ok: true,
    pages: 1,
    title: 'S20 delayed trusted typing and Control+A',
    value: { value: 'replacement', delayed: true },
    rows: [
      'context.newPage',
      'page.goto',
      'locator.focus',
      'keyboard.type',
      'keyboard.press',
      'keyboard.insertText',
      'page.evaluate',
    ],
    code: `const page = await context.newPage();
await page.goto('BASE/hardening.html');
await page.getByLabel('Shortcut input').focus();
await page.keyboard.type('abcd', { delay: 30 });
await page.keyboard.press('Control+A');
await page.keyboard.insertText('replacement');
return await page.evaluate(() => ({ value: document.querySelector('#shortcut').value, delayed: window.typedAt.length >= 4 && window.typedAt[3] - window.typedAt[0] >= 60 }));`,
  },
  {
    id: 'S21',
    ok: true,
    pages: 1,
    title: 'S21 dragTo triggers native HTML drag and drop',
    value: 'Card',
    rows: [
      'context.newPage',
      'page.goto',
      'locator.dragTo',
      'expect.toHaveText',
      'locator.textContent',
    ],
    code: `const page = await context.newPage();
await page.goto('BASE/hardening.html');
await page.locator('#drag').dragTo(page.locator('#drop'));
await expect(page.locator('#drop-result')).toHaveText('Card', { timeout: 2000 });
return await page.locator('#drop-result').textContent();`,
  },
  {
    id: 'S22',
    ok: true,
    pages: 1,
    title: 'S22 waitForResponse correlates fetch and reads JSON body',
    value: { items: ['Rust', 'JavaScript', 'CSS'] },
    rows: [
      'context.newPage',
      'page.goto',
      '{page.waitForEvent, locator.click}',
      'page.waitForEvent',
    ],
    code: `const page = await context.newPage();
await page.goto('BASE/hardening.html');
const [response] = await Promise.all([
  page.waitForResponse(response => response.url().endsWith('/api/items') && response.status() === 200),
  page.getByRole('button', { name: 'Load items' }).click(),
]);
return await response.json();`,
  },
  {
    id: 'S23',
    ok: true,
    pages: 1,
    title: 'S23 nested evaluate, evaluateAll, and full-page PNG bytes',
    value: {
      nested: {
        matrix: [[1, 2], [3]],
        options: { enabled: true, empty: null },
      },
      items: ['Rust', 'JavaScript', 'CSS'],
      png: true,
      fullPage: true,
    },
    rows: [
      'context.newPage',
      'page.goto',
      'page.evaluate',
      'locator.evaluateAll',
      'page.screenshot',
    ],
    code: `const page = await context.newPage();
await page.goto('BASE/hardening.html');
const nested = await page.evaluate(arg => ({ matrix: arg, options: { enabled: true, empty: null } }), [[1, 2], [3]]);
const items = await page.locator('#items li').evaluateAll(elements => elements.map(el => el.textContent));
const bytes = await page.screenshot({ fullPage: true });
const height = ((bytes[20] << 24) | (bytes[21] << 16) | (bytes[22] << 8) | bytes[23]) >>> 0;
return { nested, items, png: bytes.byteLength > 100 && bytes[0] === 137 && bytes[1] === 80 && bytes[2] === 78 && bytes[3] === 71, fullPage: height > 1800 };`,
  },
  {
    id: 'S24',
    ok: true,
    pages: 1,
    title: 'S24 form POST redirect waitForURL and goBack URL assertion',
    value: 'BASE/hardening.html',
    rows: [
      'context.newPage',
      'page.goto',
      '{page.waitForURL, locator.click}',
      'expect.toHaveURL',
      'page.goBack',
      'expect.toHaveURL',
    ],
    code: String.raw`const page = await context.newPage();
await page.goto('BASE/hardening.html');
await Promise.all([page.waitForURL('**/posted.html'), page.getByRole('button', { name: 'Submit redirect' }).click()]);
await expect(page).toHaveURL(/posted\.html$/);
await page.goBack();
await expect(page).toHaveURL(/hardening\.html$/);
return page.url();`,
  },
  {
    id: 'S25',
    ok: true,
    pages: 0,
    title: 'S25 pasted test body, concurrent new pages, per-page close',
    rows: [
      'context.newPage',
      'page.goto',
      'expect.toHaveTitle',
      'context.newPage',
      'context.newPage',
      'context.newPage',
      'page.goto',
      'page.goto',
      'page.goto',
      'page.close',
      'page.close',
      'page.close',
      'page.close',
    ],
    code: `test('pasted agent test', async ({ page }) => {
  await page.goto('BASE/hardening.html');
  await expect(page).toHaveTitle('Playwright hardening');
  const pages = await Promise.all([context.newPage(), context.newPage(), context.newPage()]);
  await Promise.all(pages.map(tab => tab.goto('BASE/hardening.html')));
  await Promise.all(pages.map(tab => tab.close()));
  await page.close();
});`,
  },
]

const aliases: Record<string, string> = {
  'pages.newPage': 'context.newPage',
  'pages.list': 'context.pages',
  'pages.close': 'page.close',
  'context.close': 'page.close',
  'page.keyboard.press': 'keyboard.press',
}
const errorClasses = {
  TimeoutError: /^TimeoutError\b|TimeoutError:/,
  Error: /strict mode violation/,
  SyntaxError: /SyntaxError|syntax error/i,
}

interface AuditDispatch {
  dispatchId: number
  createdAt: number
  toolName: string
  dispatchKey?: string
  parentDispatchId?: string
  resultMeta?: string
}

/** The shared fixture server only serves fixtures/pages. Keep this corpus's
 * files and listener local, using the same unrestricted 10101–20202 range.
 */
function startCorpusFixtures() {
  const directory = resolve(import.meta.dir, '../fixtures/playwright')
  let lastError: unknown
  for (let attempt = 0; attempt < 20; attempt++) {
    try {
      return Bun.serve({
        hostname: '127.0.0.1',
        port: 10101 + Math.floor(Math.random() * 10102),
        async fetch(request) {
          const { pathname } = new URL(request.url)
          if (pathname === '/api/items')
            return Response.json({ items: ['Rust', 'JavaScript', 'CSS'] })
          if (pathname === '/submit' && request.method === 'POST') {
            await request.text()
            return new Response(null, {
              status: 303,
              headers: { location: '/posted.html' },
            })
          }
          if (!/^\/[\w-]+\.(html|txt)$/.test(pathname)) {
            return new Response('not found', { status: 404 })
          }
          const file = Bun.file(resolve(directory, pathname.slice(1)))
          if (!(await file.exists()))
            return new Response('not found', { status: 404 })
          const headers: Record<string, string> = {
            'cache-control': 'no-store',
            'content-type': pathname.endsWith('.html')
              ? 'text/html; charset=utf-8'
              : 'text/plain; charset=utf-8',
          }
          if (pathname === '/notes.txt') {
            headers['content-disposition'] = 'attachment; filename="notes.txt"'
          }
          return new Response(file, { headers })
        },
      })
    } catch (error) {
      lastError = error
    }
  }
  throw new Error(`Playwright fixture server could not bind: ${lastError}`)
}

/** Consumes the corpus's ordered tokens, allowing only explicit optional,
 * repeated, or concurrent groups. Unexpected extra audit rows still fail.
 */
function assertRows(expected: string[], actual: string[], id: string): void {
  let index = 0
  for (const token of expected) {
    const message = `${id}: child rows at ${index}; expected ${token}; received ${JSON.stringify(actual)}`
    if (token.startsWith('{')) {
      const names = token
        .slice(1, -1)
        .split(',')
        .map((name) => name.trim())
      assert.deepEqual(
        actual.slice(index, index + names.length).sort(),
        names.sort(),
        message,
      )
      index += names.length
    } else if (token.startsWith('[')) {
      if (actual[index] === token.slice(1, -1)) index++
    } else if (token.endsWith('*') || token.endsWith('+')) {
      const start = index
      while (actual[index] === token.slice(0, -1)) index++
      if (token.endsWith('+')) assert.ok(index > start, message)
    } else {
      assert.equal(actual[index], token, message)
      index++
    }
  }
  assert.equal(
    index,
    actual.length,
    `${id}: extra child rows: ${actual.slice(index).join(', ')}`,
  )
}

function assertResult(
  script: CorpusScript,
  result: McpToolResult,
  base: string,
): void {
  const output = result.structuredContent
  const message = `${script.id}: playwright returned ${textOf(result)}; structured=${JSON.stringify(output)}`
  assert.equal(result.isError === true, !script.ok, message)
  assert.equal(output?.ok, script.ok, message)
  if (script.ok) {
    const expected: unknown =
      script.value === undefined
        ? undefined
        : JSON.parse(
            JSON.stringify(script.value).replaceAll('BASE/', `${base}/`),
          )
    assert.deepEqual(output?.value, expected, message)
  } else {
    assert.equal(typeof output?.error, 'string', message)
    assert.ok(script.errorClass, `${script.id}: missing expected error class`)
    const error = String(output?.error)
    assert.match(error, errorClasses[script.errorClass], message)
    for (const fragment of script.errorContains ?? []) {
      assert.ok(
        error.includes(fragment),
        `${message}; expected error fragment: ${fragment}`,
      )
    }
  }
}

async function readDispatches(
  ctx: CaseContext,
  sessionId: string,
): Promise<AuditDispatch[]> {
  // run-corpus.md §5: protocol 2025-03-26's mcp-session-id is the cockpit id.
  const response = await apiGet(
    ctx.server,
    `/api/v1/sessions/${encodeURIComponent(sessionId)}`,
  )
  if (response.status === 404) return []
  assert.equal(
    response.status,
    200,
    `Session detail returned ${response.status}: ${await response.clone().text()}`,
  )
  const detail = (await response.json()) as { dispatches: AuditDispatch[] }
  assert.ok(
    Array.isArray(detail.dispatches),
    'Session detail omitted dispatches',
  )
  return detail.dispatches.toSorted(
    (a, b) => a.createdAt - b.createdAt || a.dispatchId - b.dispatchId,
  )
}

async function assertAudit(
  ctx: CaseContext,
  sessionId: string,
  script: CorpusScript,
): Promise<void> {
  let rows: AuditDispatch[] = []
  let parent: AuditDispatch | undefined
  // Children are awaited inline; their parent goes through the audit worker.
  // Waiting for that parent avoids racing persistence without loosening order.
  await waitUntil(async () => {
    rows = await readDispatches(ctx, sessionId)
    parent = rows.findLast((row) => row.toolName === 'playwright')
    return Boolean(parent?.dispatchKey)
  }, `${script.id}: script parent to reach GET /api/v1/sessions/${sessionId}`)
  assert.ok(parent?.dispatchKey, `${script.id}: missing parent dispatch key`)
  const parentKey = parent.dispatchKey
  const children = rows.filter((row) => row.parentDispatchId === parentKey)
  const foreignChildren = rows.filter(
    (row) => row.parentDispatchId && row.parentDispatchId !== parentKey,
  )
  assert.equal(
    foreignChildren.length,
    0,
    `${script.id}: children linked to another script parent`,
  )
  assert.equal(
    JSON.parse(parent.resultMeta ?? '{}').isError,
    !script.ok,
    `${script.id}: parent isError`,
  )
  // Ignore explicit waitForTimeout rows before indexing errorRows as well as
  // matching names, so an ignored wait cannot shift which action is checked.
  const meaningful = children.filter(
    (row) => row.toolName !== 'page.waitForTimeout',
  )
  const names = meaningful.map(
    (row) =>
      aliases[row.toolName] ?? row.toolName.replace(/^assert\./, 'expect.'),
  )
  assertRows(script.rows, names, script.id)
  const expectedErrors = new Set(script.errorRows ?? [])
  for (const [index, child] of meaningful.entries()) {
    assert.equal(
      JSON.parse(child.resultMeta ?? '{}').isError,
      expectedErrors.has(index),
      `${script.id}: child ${index} (${names[index]}) isError`,
    )
  }
}

async function assertPageCount(
  ctx: CaseContext,
  sessionId: string,
  script: CorpusScript,
): Promise<void> {
  let observed: number | undefined
  // run-corpus.md §5: read the live projection before DELETE /mcp collapses
  // the agent's group. Poll its async update; do not substitute all open tabs.
  try {
    await waitUntil(async () => {
      const response = await apiGet(ctx.server, '/api/v1/sessions?status=live')
      assert.equal(
        response.status,
        200,
        `Live sessions returned ${response.status}`,
      )
      const list = (await response.json()) as {
        items: Array<{ sessionId: string; live?: { browserTabs: unknown[] } }>
      }
      observed = list.items.find((item) => item.sessionId === sessionId)?.live
        ?.browserTabs.length
      return observed === script.pages
    }, `${script.id}: live session page count to equal ${script.pages}`)
  } catch (error) {
    throw new Error(
      `${script.id}: expected ${script.pages} pages in session group; observed ${observed ?? 'no live projection'}`,
      { cause: error },
    )
  }
}

async function tabIds(session: McpSession): Promise<number[]> {
  const result = await session.callTool('tabs', { action: 'list' })
  const text = expectOk(result, 'Playwright case page cleanup inventory')
  // tabs has no output schema, so the MCP wire intentionally strips its
  // structuredContent. Its ownership sections expose one [pageId] per line.
  const pages = [...text.matchAll(/^\[(\d+)\]\s/gm)].map((match) =>
    Number(match[1]),
  )
  assert.ok(
    pages.length > 0 || text.includes('(no open pages)'),
    `Unrecognized tabs list: ${text}`,
  )
  return pages
}

async function runScript(
  ctx: CaseContext,
  script: CorpusScript,
): Promise<void> {
  const fixtures = startCorpusFixtures()
  const base = `http://127.0.0.1:${fixtures.port}`
  let session: McpSession | undefined
  let before: Set<number> | undefined
  try {
    session = await ctx.openSession(
      `playwright-corpus-${script.id.toLowerCase()}`,
    )
    assert.ok(
      session.sessionId,
      `${script.id}: MCP initialize omitted session id`,
    )
    before = new Set(await tabIds(session))
    // Prove direct corpus URLs resolve even when a not-yet-implemented tool
    // stops before navigation. Relative links/frames retain their corpus paths.
    for (const [path] of script.code.matchAll(/BASE\/[\w-]+\.html/g)) {
      const response = await fetch(path.replace('BASE', base))
      assert.equal(
        response.status,
        200,
        `${script.id}: fixture ${path} is missing`,
      )
      await response.body?.cancel()
    }
    // The harness owns scratchDir and removes it after the real browser exits.
    // Upload an actual file instead of simulating File objects in page JS.
    const uploadFile = resolve(ctx.scratchDir, 'agent-upload.txt')
    if (script.code.includes('UPLOAD_FILE'))
      await Bun.write(uploadFile, 'hello\n')
    const result = await session.callTool('playwright', {
      code: script.code
        .replaceAll('BASE/', `${base}/`)
        .replaceAll('UPLOAD_FILE', JSON.stringify(uploadFile)),
    })
    assertResult(script, result, base)
    await assertAudit(ctx, session.sessionId, script)
    await assertPageCount(ctx, session.sessionId, script)
  } finally {
    // Cases run serially in a harness-owned browser. Diff all page IDs rather
    // than only "mine" so a failed popup ownership check cannot leak a tab.
    try {
      if (session && before) {
        const cleanupSession = session
        const baseline = before
        // Closing a grouped tab can invalidate a later inventory entry. Re-list
        // after each close, and accept a missing-tab error only after proving the
        // page is gone. Teardown must not fail a passing script for an absent tab.
        await waitUntil(
          async () => {
            const page = (await tabIds(cleanupSession)).find(
              (id) => !baseline.has(id),
            )
            if (page === undefined) return true
            const closed = await cleanupSession.callTool('tabs', {
              action: 'close',
              page,
            })
            if (
              closed.isError &&
              /Unknown page|No tab with given id/.test(textOf(closed)) &&
              !(await tabIds(cleanupSession)).includes(page)
            )
              return false
            expectOk(closed, `cleanup page ${page}`)
            return false
          },
          `${script.id}: close corpus pages`,
          { timeoutMs: 5000, intervalMs: 20 },
        )
      }
    } finally {
      try {
        await session?.close()
      } finally {
        await fixtures.stop(true)
      }
    }
  }
}

export const playwrightCases: ContractCase[] = corpus.map((script) => ({
  name: `playwright: ${script.title}`,
  smoke: ['S01', 'S02', 'F13', 'F14', 'F15'].includes(script.id),
  run: (ctx) => runScript(ctx, script),
}))
