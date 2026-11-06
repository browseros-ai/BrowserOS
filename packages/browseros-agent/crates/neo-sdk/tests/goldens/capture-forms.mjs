// Records what real Playwright does for fill, clear, press, inputValue, check, uncheck, setChecked
// and selectOption on BrowserOS neo, before any Neo forms code exists, so Neo is compared against
// captured behaviour instead of hand-typed expectations.
//
//   node launches BrowserOS neo headless (fresh profile) ── CDP ──> Playwright connectOverCDP
//   node:http serves one fixture page per case; each case runs RUNS times, each on a fresh page.
//
// Each case records what a script and its page can observe: the outcome, the return value and
// type, the error's name and category, and the page after the call (control values, checked and
// selected state, the focused element, the URL, and every event the page saw). The `Input.*`
// commands Playwright sent are recorded too, from its own pw:protocol debug logger, as the
// keyboard's ground truth. Call logs and exact messages are kept for reference only.
//
// Writes form-goldens.json. Run from this directory or anywhere, under Node (Playwright's
// connectOverCDP hangs under Bun):
//
//   NEO_E2E_BROWSER="/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo" \
//   NEO_PARITY_PLAYWRIGHT=<a Playwright checkout's built packages/playwright-core> \
//   node capture-forms.mjs
//
// NEO_PARITY_PLAYWRIGHT is read exactly as ../parity/harness.mjs reads it; without it the npm
// playwright-core (1.64.1, a different revision) runs, and the goldens say so.

import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import http from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const BINARY = process.env.NEO_E2E_BROWSER;
const RUNS = 3;

if (!BINARY) {
  process.stderr.write('capture-forms: set NEO_E2E_BROWSER to a BrowserOS neo binary\n');
  process.exit(2);
}

// playwright-core reads these when it loads, so they are set before the dynamic import. The call
// log in error messages is dimmed with ANSI codes whenever the env suggests color support, even
// without a TTY; FORCE_COLOR=0 keeps the recorded text plain.
process.env.FORCE_COLOR = '0';
process.env.DEBUG = 'pw:protocol';
process.env.DEBUG_COLORS = '0';
const protocolLines = [];
const writeStderr = process.stderr.write.bind(process.stderr);
process.stderr.write = (chunk, ...rest) => {
  const text = String(chunk);
  if (text.includes('pw:protocol')) {
    protocolLines.push(text);
    return true;
  }
  return writeStderr(chunk, ...rest);
};

// The same import as ../parity/harness.mjs, plus `errors` for the TimeoutError check.
const playwrightDir = process.env.NEO_PARITY_PLAYWRIGHT;
const { chromium, errors } = await import(
  playwrightDir ? pathToFileURL(join(playwrightDir, 'index.mjs')).href : 'playwright-core'
);
const playwrightPackage = playwrightDir
  ? join(playwrightDir, 'package.json')
  : fileURLToPath(import.meta.resolve('playwright-core/package.json'));
const playwrightVersion = JSON.parse(readFileSync(playwrightPackage, 'utf8')).version;

function gitOf(dir) {
  const git = args => execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8' }).trim();
  return { commit: git(['rev-parse', 'HEAD']), dirty: git(['status', '--porcelain']).length > 0 };
}

const playwright = playwrightDir
  ? { source: 'NEO_PARITY_PLAYWRIGHT', dir: playwrightDir, version: playwrightVersion, ...gitOf(playwrightDir) }
  : { source: 'npm playwright-core', dir: dirname(playwrightPackage), version: playwrightVersion, commit: null };

// Registered first, in the capture phase on window, so it sees every event even if a later
// listener stops it. Events persist in sessionStorage, so a form submission's navigation keeps
// the events that led to it; a `document` entry marks each page load and `call` the call's start.
const RECORDER = `<script>
(() => {
  const events = JSON.parse(sessionStorage.getItem('__events') || '[]');
  window.__events = events;
  window.__record = event => {
    events.push(event);
    sessionStorage.setItem('__events', JSON.stringify(events));
  };
  const name = t => t === window ? 'window' : t === document ? 'document' : (t && (t.id || (t.tagName && t.tagName.toLowerCase()))) || String(t);
  const value = t => {
    if (!t || t === window || t === document) return null;
    if (t.type === 'checkbox' || t.type === 'radio') return t.checked;
    if (t instanceof HTMLSelectElement) return [...t.selectedOptions].map(o => o.value);
    if (t.isContentEditable) return t.textContent;
    return 'value' in t ? t.value : null;
  };
  __record({ type: 'document', url: location.pathname + location.search });
  for (const type of ['focus', 'blur', 'keydown', 'keypress', 'keyup', 'beforeinput', 'input', 'change', 'select', 'click', 'submit'])
    addEventListener(type, e => {
      const event = { type: e.type, target: name(e.target), isTrusted: e.isTrusted };
      if (e instanceof KeyboardEvent)
        Object.assign(event, { key: e.key, code: e.code, keyCode: e.keyCode, charCode: e.charCode, location: e.location, repeat: e.repeat, modifiers: [e.altKey && 'Alt', e.ctrlKey && 'Control', e.metaKey && 'Meta', e.shiftKey && 'Shift'].filter(Boolean).join('+') });
      if (e instanceof InputEvent)
        Object.assign(event, { inputType: e.inputType, data: e.data });
      if (e.type === 'beforeinput' || e.type === 'input' || e.type === 'change')
        event.value = value(e.target);
      if (e.type === 'click')
        event.detail = e.detail;
      if (e.type === 'submit')
        event.submitter = e.submitter ? name(e.submitter) : null;
      __record(event);
    }, true);
})();
</script>`;

const FORM = `<form action="/submitted" method="get"><input id="q" name="q" value="neo"></form>`;
const COLORS = `<select id="color"><option value="r">Red</option><option value="g">Green</option><option value="b">Blue</option></select>`;
const OVERLAY = `<div id="overlay" style="position:fixed;top:0;left:0;width:100vw;height:100vh;z-index:10;background:rgba(0,0,0,0.3)"></div>`;
const FOCUSED_AT_END = `<input id="name" value="abc"><script>document.getElementById('name').focus(); document.getElementById('name').setSelectionRange(3, 3);</script>`;
const LAYOUT_KEYS = ['Shift+a', 'Shift+KeyB', '+', 'Shift++', 'Shift+Digit1', 'Shift+Numpad1', 'Numpad1', 'Space'];

// `source` is the agent-side code, run as an async function of `page`. `keys` lists every key the
// case presses, in order, for neo-runtime's keyboard check. `expect` is what this script requires
// of the outcome; a case that behaves otherwise fails the capture, so a surprise is looked at.
const CASES = [
  // fill and clear
  { name: 'fill', method: 'fill', expect: 'resolved', body: `<input id="name">`,
    source: `return await page.locator('#name').fill('hello');` },
  { name: 'fill-twice', method: 'fill', expect: 'resolved', body: `<input id="name">`,
    source: `await page.locator('#name').fill('first');\nreturn await page.locator('#name').fill('second');` },
  { name: 'clear', method: 'clear', expect: 'resolved', body: `<input id="name" value="prefilled">`, keys: ['Delete'],
    source: `return await page.locator('#name').clear();` },
  { name: 'fill-empty', method: 'fill', expect: 'resolved', body: `<input id="name" value="prefilled">`, keys: ['Delete'],
    source: `return await page.locator('#name').fill('');` },
  { name: 'fill-textarea', method: 'fill', expect: 'resolved', body: `<textarea id="bio">old</textarea>`,
    source: `return await page.locator('#bio').fill('line 1\\nline 2');` },
  { name: 'fill-contenteditable', method: 'fill', expect: 'resolved', body: `<div id="editor" contenteditable="true">old text</div>`,
    source: `return await page.locator('#editor').fill('new text');` },
  { name: 'fill-date', method: 'fill', expect: 'resolved', body: `<input id="when" type="date">`,
    source: `return await page.locator('#when').fill('2026-11-06');` },
  { name: 'fill-number-text', method: 'fill', expect: 'threw', body: `<input id="qty" type="number">`,
    source: `return await page.locator('#qty').fill('abc');` },
  { name: 'fill-checkbox', method: 'fill', expect: 'threw', body: `<input id="agree" type="checkbox">`,
    source: `return await page.locator('#agree').fill('x');` },
  { name: 'clear-checkbox', method: 'clear', expect: 'threw', body: `<input id="agree" type="checkbox">`,
    source: `return await page.locator('#agree').clear();` },
  { name: 'fill-disabled', method: 'fill', expect: 'threw', body: `<input id="name" disabled>`,
    source: `return await page.locator('#name').fill('x', { timeout: 500 });` },
  { name: 'fill-readonly', method: 'fill', expect: 'threw', body: `<input id="name" readonly value="fixed">`,
    source: `return await page.locator('#name').fill('x', { timeout: 500 });` },
  { name: 'fill-missing', method: 'fill', expect: 'threw', body: `<input id="name">`,
    source: `return await page.locator('#missing').fill('x', { timeout: 500 });` },
  { name: 'fill-strict', method: 'fill', expect: 'threw', body: `<input id="first"><input id="second">`,
    source: `return await page.locator('input').fill('x');` },
  { name: 'fill-get-by-label', method: 'fill', expect: 'resolved', body: `<label for="email">Email</label><input id="email">`,
    source: `return await page.getByLabel('Email').fill('neo@example.com');` },
  { name: 'fill-label-element', method: 'fill', expect: 'resolved', body: `<label for="email">Email</label><input id="email">`,
    source: `return await page.locator('label').fill('neo@example.com');` },
  { name: 'page-fill', method: 'fill', expect: 'resolved', body: `<input id="name">`,
    source: `return await page.fill('#name', 'via page');` },

  // press
  { name: 'press-enter', method: 'press', expect: 'resolved', body: FORM, keys: ['Enter'],
    source: `await page.locator('#q').press('Enter');\nreturn page.url();` },
  { name: 'press-enter-no-wait', method: 'press', expect: 'resolved', body: FORM, keys: ['Enter'],
    source: `await page.locator('#q').press('Enter', { noWaitAfter: true });\nreturn page.url();`,
    settle: page => page.waitForURL(/submitted/) },
  { name: 'press-shift-a', method: 'press', expect: 'resolved', body: `<input id="name">`, keys: ['Shift+A'],
    source: `return await page.locator('#name').press('Shift+A');` },
  { name: 'press-select-all-backspace', method: 'press', expect: 'resolved', body: `<input id="name" value="hello world">`,
    keys: ['ControlOrMeta+a', 'Backspace'],
    source: `await page.locator('#name').press('ControlOrMeta+a');\nreturn await page.locator('#name').press('Backspace');` },
  { name: 'press-arrow-left', method: 'press', expect: 'resolved', body: FOCUSED_AT_END, keys: ['ArrowLeft'],
    source: `return await page.locator('#name').press('ArrowLeft');` },
  { name: 'press-delay', method: 'press', expect: 'resolved', body: `<input id="name">`, keys: ['a'],
    source: `return await page.locator('#name').press('a', { delay: 50 });` },
  { name: 'press-layout', method: 'press', expect: 'resolved', body: `<input id="name">`, keys: LAYOUT_KEYS,
    source: `for (const key of ${JSON.stringify(LAYOUT_KEYS).replaceAll('"', "'").replaceAll(',', ', ')})\n  await page.locator('#name').press(key);` },
  { name: 'press-unknown', method: 'press', expect: 'threw', body: `<input id="name">`, keys: ['Foo'],
    source: `return await page.locator('#name').press('Foo');` },
  { name: 'press-shift-unknown', method: 'press', expect: 'threw', body: `<input id="name">`, keys: ['Shift+Foo'],
    source: `return await page.locator('#name').press('Shift+Foo');` },
  { name: 'page-press-tab', method: 'press', expect: 'resolved', body: `<input id="first"><input id="second">`, keys: ['Tab'],
    source: `return await page.press('#first', 'Tab');` },

  // inputValue
  { name: 'input-value', method: 'inputValue', expect: 'resolved', body: `<input id="name" value="hello">`,
    source: `return await page.locator('#name').inputValue();` },
  { name: 'input-value-textarea', method: 'inputValue', expect: 'resolved', body: `<textarea id="bio">some text</textarea>`,
    source: `return await page.locator('#bio').inputValue();` },
  { name: 'input-value-select', method: 'inputValue', expect: 'resolved',
    body: `<select id="color"><option value="r">Red</option><option value="g" selected>Green</option></select>`,
    source: `return await page.locator('#color').inputValue();` },
  { name: 'input-value-label', method: 'inputValue', expect: 'resolved', body: `<label for="email">Email</label><input id="email" value="neo@example.com">`,
    source: `return await page.locator('label').inputValue();` },
  { name: 'input-value-div', method: 'inputValue', expect: 'threw', body: `<div id="box">text</div>`,
    source: `return await page.locator('#box').inputValue();` },
  { name: 'input-value-missing', method: 'inputValue', expect: 'threw', body: `<input id="name">`,
    source: `return await page.locator('#missing').inputValue({ timeout: 500 });` },

  // check, uncheck, setChecked
  { name: 'check', method: 'check', expect: 'resolved', body: `<input id="agree" type="checkbox">`,
    source: `return await page.locator('#agree').check();` },
  { name: 'uncheck', method: 'uncheck', expect: 'resolved', body: `<input id="agree" type="checkbox" checked>`,
    source: `return await page.locator('#agree').uncheck();` },
  { name: 'check-already-checked', method: 'check', expect: 'resolved', body: `<input id="agree" type="checkbox" checked>`,
    source: `return await page.locator('#agree').check();` },
  { name: 'check-radio', method: 'check', expect: 'resolved', body: `<input id="small" type="radio" name="size" checked><input id="large" type="radio" name="size">`,
    source: `return await page.locator('#large').check();` },
  { name: 'uncheck-radio', method: 'uncheck', expect: 'threw', body: `<input id="small" type="radio" name="size" checked>`,
    source: `return await page.locator('#small').uncheck();` },
  { name: 'check-role-checkbox', method: 'check', expect: 'resolved',
    body: `<div id="custom" role="checkbox" aria-checked="false" tabindex="0" onclick="this.setAttribute('aria-checked', String(this.getAttribute('aria-checked') !== 'true'))">Custom</div>`,
    source: `return await page.locator('#custom').check();` },
  { name: 'check-prevented', method: 'check', expect: 'threw', body: `<input id="agree" type="checkbox" onclick="event.preventDefault()">`,
    source: `return await page.locator('#agree').check();` },
  { name: 'check-div', method: 'check', expect: 'threw', body: `<div id="box">text</div>`,
    source: `return await page.locator('#box').check();` },
  { name: 'check-covered', method: 'check', expect: 'threw', body: `<input id="agree" type="checkbox">${OVERLAY}`,
    source: `return await page.locator('#agree').check({ timeout: 1000 });` },
  { name: 'check-trial', method: 'check', expect: 'resolved', body: `<input id="agree" type="checkbox">`,
    source: `return await page.locator('#agree').check({ trial: true });` },
  { name: 'set-checked-true', method: 'setChecked', expect: 'resolved', body: `<input id="agree" type="checkbox">`,
    source: `return await page.locator('#agree').setChecked(true);` },
  { name: 'set-checked-false', method: 'setChecked', expect: 'resolved', body: `<input id="agree" type="checkbox" checked>`,
    source: `return await page.locator('#agree').setChecked(false);` },
  { name: 'set-checked-div', method: 'setChecked', expect: 'threw', body: `<div id="box">text</div>`,
    source: `return await page.locator('#box').setChecked(true);` },

  // selectOption
  { name: 'select-value', method: 'selectOption', expect: 'resolved', body: COLORS,
    source: `return await page.locator('#color').selectOption('g');` },
  { name: 'select-label', method: 'selectOption', expect: 'resolved', body: COLORS,
    source: `return await page.locator('#color').selectOption('Blue');` },
  { name: 'select-label-object', method: 'selectOption', expect: 'resolved', body: COLORS,
    source: `return await page.locator('#color').selectOption({ label: 'Red' });` },
  { name: 'select-index', method: 'selectOption', expect: 'resolved', body: COLORS,
    source: `return await page.locator('#color').selectOption({ index: 2 });` },
  { name: 'select-value-object', method: 'selectOption', expect: 'resolved', body: COLORS,
    source: `return await page.locator('#color').selectOption({ value: 'g' });` },
  { name: 'select-multiple', method: 'selectOption', expect: 'resolved',
    body: `<select id="pets" multiple><option value="cat">Cat</option><option value="dog">Dog</option><option value="fish">Fish</option></select>`,
    source: `return await page.locator('#pets').selectOption(['cat', 'fish']);` },
  { name: 'select-missing-option', method: 'selectOption', expect: 'threw', body: COLORS,
    source: `return await page.locator('#color').selectOption('purple', { timeout: 500 });` },
  { name: 'select-disabled-option', method: 'selectOption', expect: 'threw',
    body: `<select id="color"><option value="r">Red</option><option value="x" disabled>Unavailable</option></select>`,
    source: `return await page.locator('#color').selectOption('x', { timeout: 500 });` },
  { name: 'select-not-a-select', method: 'selectOption', expect: 'threw', body: `<div id="box">text</div>`,
    source: `return await page.locator('#box').selectOption('x');` },
  { name: 'select-null', method: 'selectOption', expect: 'resolved',
    body: `<select id="color"><option value="r">Red</option><option value="g" selected>Green</option></select>`,
    source: `return await page.locator('#color').selectOption(null);` },
];

const fixtureHtml = c => `<!doctype html><html><head><meta charset="utf-8"><title>${c.name}</title>${RECORDER}</head><body>${c.body}</body></html>`;
const SUBMITTED_HTML = `<!doctype html><html><head><meta charset="utf-8"><title>submitted</title>${RECORDER}</head><body></body></html>`;

const AsyncFunction = (async () => {}).constructor;

// The behaviour a parity check compares; `firstLine` and `message` are for people.
function describeError(e) {
  const message = String(e?.message ?? e);
  return {
    name: e?.name ?? null,
    isTimeout: e instanceof errors.TimeoutError,
    category: categorize(e, message),
    apiName: /^([\w.]+): /.exec(message)?.[1] ?? null,
    firstLine: message.split('\n')[0],
    message,
  };
}

function categorize(e, message) {
  if (e instanceof errors.TimeoutError)
    return 'timeout';
  const categories = [
    ['strict', /strict mode violation/],
    ['unknown-key', /Unknown key: /],
    ['wrong-element', /Not a checkbox or radio button|is not a <select> element|is not an <input>, <textarea> or <select> element|is not an <input>, <textarea> or \[contenteditable\] element|Input of type "[^"]*" cannot be filled/],
    ['invalid-value', /Cannot type text into input\[type=number\]|Malformed value/],
    ['radio-uncheck', /Cannot uncheck radio button/],
    ['state-unchanged', /did not change its state/],
  ];
  return categories.find(([, pattern]) => pattern.test(message.split('\nCall log:')[0]))?.[0] ?? 'other';
}

const SEND = 'SEND ► ';

// Only Playwright's own `Input.*` sends are kept: their params are the keyboard's ground truth.
function inputSends(lines) {
  const sends = [];
  for (const line of lines) {
    const clean = line.replace(/\x1b\[[0-9;]*m/g, '').trim();
    const at = clean.indexOf(SEND);
    if (at < 0)
      continue;
    let message;
    try {
      message = JSON.parse(clean.slice(at + SEND.length));
    } catch {
      continue;
    }
    if (message.method?.startsWith('Input.'))
      sends.push({ method: message.method, params: message.params });
  }
  return sends;
}

async function pageFacts(page, origin) {
  const facts = await page.evaluate(() => {
    const name = t => !t ? null : t === document.body ? 'body' : t.id || t.tagName.toLowerCase();
    const controls = [...document.querySelectorAll('input, textarea, select, [contenteditable], [role=checkbox], [role=radio]')].map(el => {
      const control = { id: el.id || null, tag: el.tagName.toLowerCase() };
      if (el instanceof HTMLInputElement) {
        control.type = el.type;
        if (el.type === 'checkbox' || el.type === 'radio')
          control.checked = el.checked;
        else
          control.value = el.value;
        if (el.selectionStart !== null)
          control.selection = [el.selectionStart, el.selectionEnd];
      } else if (el instanceof HTMLTextAreaElement) {
        control.value = el.value;
        control.selection = [el.selectionStart, el.selectionEnd];
      } else if (el instanceof HTMLSelectElement) {
        control.multiple = el.multiple;
        control.selected = [...el.selectedOptions].map(o => o.value);
        control.selectedIndex = el.selectedIndex;
      } else if (el.isContentEditable) {
        control.text = el.textContent;
      }
      if (el.hasAttribute('role')) {
        control.role = el.getAttribute('role');
        control.ariaChecked = el.getAttribute('aria-checked');
      }
      return control;
    });
    return { url: location.href, title: document.title, activeElement: name(document.activeElement), controls, events: window.__events ?? null };
  });
  return { ...facts, urlRelative: relative(facts.url, origin) };
}

const relative = (value, origin) => typeof value === 'string' && value.startsWith(origin) ? value.slice(origin.length) : value;

async function runCase(context, origin, c) {
  const act = new AsyncFunction('page', c.source);
  const page = await context.newPage();
  try {
    await page.goto(`${origin}/${c.name}.html`);
    await page.evaluate(() => window.__record({ type: 'call' }));
    const start = protocolLines.length;
    const started = performance.now();
    let result;
    try {
      const value = await act(page);
      result = { outcome: 'resolved', returnType: typeof value };
      if (value !== undefined)
        Object.assign(result, { returnValue: value, returnValueRelative: relative(value, origin) });
    } catch (e) {
      result = { outcome: 'threw', error: describeError(e) };
    }
    const elapsedMs = Math.round(performance.now() - started);
    const input = inputSends(protocolLines.slice(start));
    if (c.settle)
      await c.settle(page);
    return { ...result, page: await pageFacts(page, origin), input, elapsedMs };
  } finally {
    await page.close();
  }
}

// What must be the same on every run for a case to count as deterministic: what a script and its
// page observe. Retry call logs and timings are left out; they follow Playwright's backoff.
function signature(run) {
  const { error } = run;
  return JSON.stringify({
    outcome: run.outcome,
    returnType: run.returnType,
    returnValue: run.returnValueRelative,
    error: error && { name: error.name, category: error.category, apiName: error.apiName, firstLine: error.firstLine },
    page: { ...run.page, url: undefined },
    input: run.input,
  });
}

function collectDescendants(rootPid) {
  const rows = execFileSync('ps', ['-Ao', 'pid=,ppid='], { encoding: 'utf8' })
    .trim().split('\n').map(l => l.trim().split(/\s+/).map(Number));
  const found = new Set([rootPid]);
  for (let grew = true; grew;) {
    grew = false;
    for (const [pid, ppid] of rows) {
      if (found.has(ppid) && !found.has(pid)) {
        found.add(pid);
        grew = true;
      }
    }
  }
  return [...found];
}

const isAlive = pid => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};

const processesMentioning = path => execFileSync('ps', ['-Ao', 'pid=,command='], { encoding: 'utf8' })
  .split('\n').filter(l => l.includes(path)).map(l => l.trim());

const LAUNCH_ARGS = ['--headless=new', '--disable-browseros-server', '--remote-debugging-port=0', '--no-first-run'];

async function launchBrowserOS(profile) {
  const child = spawn(BINARY, [...LAUNCH_ARGS, `--user-data-dir=${profile}`, 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] });
  let stderrTail = '';
  child.stderr.on('data', d => { stderrTail = (stderrTail + d).slice(-4000); });
  const exited = new Promise(resolve => child.once('exit', resolve));
  const portFile = join(profile, 'DevToolsActivePort');
  const deadline = Date.now() + 30_000;
  while (!existsSync(portFile) || !readFileSync(portFile, 'utf8').includes('\n')) {
    if (Date.now() > deadline || child.exitCode !== null) {
      child.kill('SIGKILL');
      throw new Error(`BrowserOS neo did not write DevToolsActivePort.\n${stderrTail}`);
    }
    await new Promise(r => setTimeout(r, 50));
  }
  const port = Number(readFileSync(portFile, 'utf8').split('\n')[0]);
  return { child, exited, port };
}

// Disconnecting Playwright leaves a connectOverCDP browser running, so the process tree is killed
// here, and every pid seen before shutdown, plus anything still naming the profile, must be gone.
async function shutdownBrowserOS({ child, exited }, profile) {
  const tree = collectDescendants(child.pid);
  child.kill('SIGTERM');
  const timedOut = await Promise.race([exited.then(() => false), new Promise(r => setTimeout(() => r(true), 5000))]);
  if (timedOut)
    child.kill('SIGKILL');
  for (let i = 0; i < 50 && tree.some(isAlive); i++)
    await new Promise(r => setTimeout(r, 100));
  for (const pid of tree.filter(isAlive))
    process.kill(pid, 'SIGKILL');
  return {
    processTreeAtShutdown: tree.length,
    signal: timedOut ? 'SIGKILL' : 'SIGTERM',
    survivingPids: tree.filter(isAlive),
    survivingProfileProcesses: processesMentioning(profile),
  };
}

function appVersion() {
  const plist = join(BINARY, '../../Info.plist');
  if (!existsSync(plist))
    return null;
  return execFileSync('plutil', ['-extract', 'CFBundleShortVersionString', 'raw', plist], { encoding: 'utf8' }).trim();
}

const tmp = mkdtempSync(join(tmpdir(), 'pw-form-goldens-'));
const profile = join(tmp, 'profile');
const server = http.createServer((req, res) => {
  const path = req.url.split('?')[0];
  const c = CASES.find(k => path === `/${k.name}.html`);
  const html = c ? fixtureHtml(c) : path === '/submitted' ? SUBMITTED_HTML : null;
  res.writeHead(html ? 200 : 404, { 'content-type': 'text/html; charset=utf-8' });
  res.end(html ?? 'not found');
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const origin = `http://127.0.0.1:${server.address().port}`;

let launched;
let browser;
let cleanup;
let versions;
const results = [];
const surprises = [];
try {
  launched = await launchBrowserOS(profile);
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${launched.port}`);
  const cdp = await browser.newBrowserCDPSession();
  const browserVersion = await cdp.send('Browser.getVersion');
  await cdp.detach();
  versions = {
    playwright,
    browserOSApp: appVersion(),
    browserVersion: browser.version(),
    cdpBrowserGetVersion: browserVersion,
    node: process.version,
  };

  const context = browser.contexts()[0];
  for (const c of CASES) {
    const runs = [];
    for (let i = 0; i < RUNS; i++)
      runs.push(await runCase(context, origin, c));
    const variants = [...new Set(runs.map(signature))];
    const { elapsedMs, ...first } = runs[0];
    results.push({
      name: c.name,
      method: c.method,
      source: c.source,
      ...(c.keys ? { keys: c.keys } : {}),
      fixtureBody: c.body,
      fixtureHtml: fixtureHtml(c),
      ...first,
      elapsedMsPerRun: runs.map(r => r.elapsedMs),
      identicalAcrossRuns: variants.length === 1,
      ...(variants.length > 1 ? { variants: variants.map(v => JSON.parse(v)) } : {}),
    });
    if (first.outcome !== c.expect)
      surprises.push(`${c.name}: expected ${c.expect}, got ${first.outcome}`);
    const summary = first.outcome === 'threw'
      ? `${first.error.name} [${first.error.category}] ${first.error.firstLine}`
      : `${first.returnType} ${JSON.stringify(first.returnValueRelative ?? null)}`;
    process.stdout.write(`${c.name}: ${first.outcome} ${summary}${variants.length > 1 ? ' (runs differ)' : ''}\n`);
  }
} finally {
  await browser?.close().catch(() => {});
  if (launched)
    cleanup = await shutdownBrowserOS(launched, profile);
  await new Promise(r => server.close(r));
  rmSync(tmp, { recursive: true, force: true });
}

const cleanupOk = cleanup.survivingPids.length === 0 && cleanup.survivingProfileProcesses.length === 0;
writeFileSync(join(HERE, 'form-goldens.json'), JSON.stringify({
  capturedAt: new Date().toISOString(),
  versions,
  launch: { binary: BINARY, args: [...LAUNCH_ARGS, '--user-data-dir=<fresh temp dir>', 'about:blank'] },
  serverOrigin: origin,
  colorMode: 'FORCE_COLOR=0: call log lines carry no ANSI dim codes',
  runsPerCase: RUNS,
  cleanup: { ...cleanup, profileRemoved: !existsSync(tmp) },
  cases: results,
}, null, 2) + '\n');
process.stdout.write(`playwright ${playwright.version} (${playwright.source}${playwright.commit ? ` ${playwright.commit}` : ''})\n`);
process.stdout.write(`${results.length} cases, ${results.filter(r => !r.identicalAcrossRuns).length} differing across runs\n`);
process.stdout.write(`cleanup: ${cleanup.processTreeAtShutdown} processes stopped by ${cleanup.signal}, survivors ${cleanup.survivingPids.length}, profile processes ${cleanup.survivingProfileProcesses.length}\n`);
for (const surprise of surprises)
  process.stdout.write(`surprise: ${surprise}\n`);
if (!cleanupOk || surprises.length)
  process.exitCode = 1;
