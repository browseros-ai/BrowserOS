// Records what real Playwright does when it clicks through a locator on BrowserOS neo, so Neo's
// implementation is compared against captured truth instead of hand-typed expectations.
//
//   node launches BrowserOS neo headless (fresh profile) ── CDP ──> playwright-core 1.64.1 connectOverCDP
//   node:http serves one fixture page per case; each case runs RUNS times, each on a fresh page.
//
// Writes click-goldens.json. When selector-goldens.json exists (bun selector-corpus.ts), it also
// evaluates every chain against this real Playwright and writes selector-crosscheck.json.
// CDP traffic comes from Playwright's own pw:protocol debug logger, captured synchronously off stderr,
// so the slice between "click started" and "click settled" is exactly the click's traffic.
//
// Run: node capture-click.mjs

import { spawn, execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import http from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const APP = '/Applications/BrowserOS neo.app';
const BINARY = `${APP}/Contents/MacOS/BrowserOS neo`;
const RUNS = 3;
const MAX_CDP_MESSAGES = 400;

// playwright-core reads these when it loads, so they must be set before the dynamic import.
// The call log in error messages is dimmed with ANSI codes whenever the env suggests color support
// (COLORTERM, TERM=xterm-*), even without a TTY. FORCE_COLOR=0 pins the plain text so goldens don't
// depend on the terminal they were captured in.
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
const { chromium, errors } = await import('playwright-core');
const playwrightVersion = JSON.parse(readFileSync(join(HERE, 'node_modules/playwright-core/package.json'), 'utf8')).version;

const SUBMIT = `page.getByRole('button', { name: 'Submit' })`;

// The recorder is part of every fixture so input reaching the page is observable without extra CDP calls.
// It registers first, in the capture phase on window, so it sees events even if a later listener stops them.
const RECORDER = `<script>
window.__events = [];
for (const type of ['pointermove', 'pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click'])
  addEventListener(type, e => __events.push({ type: e.type, target: e.target.id || e.target.tagName.toLowerCase(), isTrusted: e.isTrusted, clientX: e.clientX, clientY: e.clientY, button: e.button, buttons: e.buttons, detail: e.detail }), true);
</script>`;

const CASES = [
  {
    name: 'ok',
    body: `<button onclick="location.hash='done'">Submit</button>`,
    source: `await ${SUBMIT}.click(); return page.url();`,
    act: async page => {
      const resolved = await page.getByRole('button', { name: 'Submit' }).click();
      return { resolved, url: page.url() };
    },
  },
  {
    name: 'strict',
    body: `<button>Submit</button><button>Submit</button>`,
    source: `await ${SUBMIT}.click();`,
    act: page => page.getByRole('button', { name: 'Submit' }).click(),
  },
  {
    name: 'hidden',
    body: `<button style="display:none">Submit</button>`,
    source: `await ${SUBMIT}.click({ timeout: 500 });`,
    act: page => page.getByRole('button', { name: 'Submit' }).click({ timeout: 500 }),
  },
  {
    name: 'covered',
    body: `<button>Submit</button><div id="overlay" style="position:fixed;top:0;left:0;width:100vw;height:100vh;z-index:10;background:rgba(0,0,0,0.3)"></div>`,
    source: `await ${SUBMIT}.click({ timeout: 1000 });`,
    act: page => page.getByRole('button', { name: 'Submit' }).click({ timeout: 1000 }),
  },
  {
    name: 'disabled',
    body: `<button disabled>Submit</button>`,
    source: `await ${SUBMIT}.click({ timeout: 500 });`,
    act: page => page.getByRole('button', { name: 'Submit' }).click({ timeout: 500 }),
  },
];

const fixtureHtml = c => `<!doctype html><html><head><meta charset="utf-8"><title>${c.name}</title>${RECORDER}</head><body>${c.body}</body></html>`;

// className is whatever the playwright-core bundle named the class (1.64.1 emits TimeoutError2), so
// name and instanceOfTimeoutError are the fields that identify the error kind.
function describeError(e) {
  return {
    className: e.constructor.name,
    name: e.name,
    instanceOfTimeoutError: e instanceof errors.TimeoutError,
    firstLine: e.message.split('\n')[0],
    message: e.message,
    messageSplit: e.message.split('Call log:'),
  };
}

function truncateStrings(value) {
  if (typeof value === 'string')
    return value.length > 240 ? `${value.slice(0, 240)}...[+${value.length - 240} chars]` : value;
  if (Array.isArray(value))
    return value.map(truncateStrings);
  if (value && typeof value === 'object')
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, truncateStrings(v)]));
  return value;
}

const SEND = 'SEND ► ';
const RECV = '◀ RECV ';

function parseProtocolLine(line) {
  const clean = line.replace(/\x1b\[[0-9;]*m/g, '').trim();
  const send = clean.indexOf(SEND);
  const recv = clean.indexOf(RECV);
  const dir = send >= 0 ? 'send' : 'recv';
  const json = send >= 0 ? clean.slice(send + SEND.length) : clean.slice(recv + RECV.length);
  try {
    const { sessionId, ...message } = JSON.parse(json);
    return { dir, session: sessionId ? sessionId.slice(0, 8) : 'browser', ...truncateStrings(message) };
  } catch {
    // Playwright truncates very long messages in the log itself (MAX_LOG_LENGTH), leaving invalid JSON.
    return { dir, truncatedByPlaywright: true, method: /"method":"([^"]+)"/.exec(json)?.[1] ?? null };
  }
}

async function pageFacts(page) {
  return await page.evaluate(() => {
    const buttons = [...document.querySelectorAll('button')].map(b => {
      const r = b.getBoundingClientRect();
      const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
      return {
        outerHTML: b.outerHTML,
        box: { x: r.x, y: r.y, width: r.width, height: r.height },
        elementAtCenter: hit ? hit.id || hit.tagName.toLowerCase() : null,
      };
    });
    return {
      href: location.href,
      hash: location.hash,
      activeElement: document.activeElement?.tagName.toLowerCase() ?? null,
      buttons,
      events: window.__events,
    };
  });
}

async function runCase(context, origin, c, withFacts) {
  const page = await context.newPage();
  try {
    await page.goto(`${origin}/${c.name}.html`);
    const cdpStart = protocolLines.length;
    const started = performance.now();
    let outcome;
    try {
      const value = await c.act(page);
      outcome = { outcome: 'resolved', resolvedType: typeof value?.resolved, url: value?.url, urlRelative: value?.url?.slice(origin.length) };
    } catch (e) {
      outcome = { outcome: 'threw', error: describeError(e) };
    }
    const elapsedMs = Math.round(performance.now() - started);
    const cdpLines = protocolLines.slice(cdpStart);
    if (!withFacts)
      return { ...outcome, elapsedMs };
    return {
      ...outcome,
      elapsedMs,
      page: await pageFacts(page),
      cdpMessageCount: cdpLines.length,
      cdp: cdpLines.slice(0, MAX_CDP_MESSAGES).map(parseProtocolLine),
    };
  } finally {
    await page.close();
  }
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

async function launchBrowserOS(profile) {
  const child = spawn(BINARY, [
    '--headless=new',
    '--disable-browseros-server',
    '--remote-debugging-port=0',
    `--user-data-dir=${profile}`,
    '--no-first-run',
    'about:blank',
  ], { stdio: ['ignore', 'ignore', 'pipe'] });
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

// Disconnecting Playwright leaves a connectOverCDP browser running, so the process tree is killed here
// and every pid seen before shutdown, plus anything still naming the profile, must be gone afterwards.
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

function crossCheckSelectors(page) {
  const goldensPath = join(HERE, 'selector-goldens.json');
  if (!existsSync(goldensPath))
    return null;
  const goldens = JSON.parse(readFileSync(goldensPath, 'utf8'));
  const mismatches = [];
  for (const { chain, selector } of goldens) {
    const real = new Function('page', `return ${chain};`)(page)._selector;
    if (real !== selector)
      mismatches.push({ chain, mirror: selector, playwright: real });
  }
  return { playwrightVersion, total: goldens.length, matched: goldens.length - mismatches.length, mismatches };
}

const tmp = mkdtempSync(join(tmpdir(), 'pw-click-goldens-'));
const profile = join(tmp, 'profile');
const server = http.createServer((req, res) => {
  const c = CASES.find(k => req.url === `/${k.name}.html`);
  res.writeHead(c ? 200 : 404, { 'content-type': 'text/html; charset=utf-8' });
  res.end(c ? fixtureHtml(c) : 'not found');
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const origin = `http://127.0.0.1:${server.address().port}`;

let launched;
let browser;
let cleanup;
const results = [];
let crossCheck = null;
let versions;
try {
  launched = await launchBrowserOS(profile);
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${launched.port}`);
  const cdp = await browser.newBrowserCDPSession();
  const browserVersion = await cdp.send('Browser.getVersion');
  await cdp.detach();
  const plist = `${APP}/Contents/Info.plist`;
  versions = {
    playwright: playwrightVersion,
    browserOSApp: execFileSync('plutil', ['-extract', 'CFBundleShortVersionString', 'raw', plist], { encoding: 'utf8' }).trim(),
    browserVersion: browser.version(),
    cdpBrowserGetVersion: browserVersion,
    node: process.version,
  };

  const context = browser.contexts()[0];
  for (const c of CASES) {
    const runs = [];
    for (let i = 0; i < RUNS; i++)
      runs.push(await runCase(context, origin, c, i === 0));
    const signature = r => JSON.stringify(r.outcome === 'threw' ? r.error : { resolvedType: r.resolvedType, urlRelative: r.urlRelative });
    const variants = [...new Set(runs.map(signature))];
    const { elapsedMs, ...first } = runs[0];
    results.push({
      name: c.name,
      source: c.source,
      fixtureBody: c.body,
      fixtureHtml: fixtureHtml(c),
      ...first,
      elapsedMsPerRun: runs.map(r => r.elapsedMs),
      identicalAcrossRuns: variants.length === 1,
      ...(variants.length > 1 ? { variants: variants.map(v => JSON.parse(v)) } : {}),
    });
    process.stdout.write(`${c.name}: ${first.outcome === 'threw' ? first.error.firstLine : `resolved ${first.resolvedType}, ${first.url}`}\n`);
  }

  const page = await context.newPage();
  crossCheck = crossCheckSelectors(page);
  await page.close();
} finally {
  await browser?.close().catch(() => {});
  if (launched)
    cleanup = await shutdownBrowserOS(launched, profile);
  await new Promise(r => server.close(r));
  rmSync(tmp, { recursive: true, force: true });
}

const cleanupOk = cleanup.survivingPids.length === 0 && cleanup.survivingProfileProcesses.length === 0;
writeFileSync(join(HERE, 'click-goldens.json'), JSON.stringify({
  capturedAt: new Date().toISOString(),
  versions,
  launch: { binary: BINARY, args: ['--headless=new', '--disable-browseros-server', '--remote-debugging-port=0', '--user-data-dir=<fresh temp dir>', '--no-first-run', 'about:blank'] },
  serverOrigin: origin,
  colorMode: 'FORCE_COLOR=0: call log lines carry no ANSI dim codes',
  runsPerCase: RUNS,
  cleanup: { ...cleanup, profileRemoved: !existsSync(tmp) },
  cases: results,
}, null, 2) + '\n');
process.stdout.write(`cleanup: ${cleanup.processTreeAtShutdown} processes stopped by ${cleanup.signal}, survivors ${cleanup.survivingPids.length}, profile processes ${cleanup.survivingProfileProcesses.length}\n`);

if (crossCheck) {
  writeFileSync(join(HERE, 'selector-crosscheck.json'), JSON.stringify(crossCheck, null, 2) + '\n');
  process.stdout.write(`selector cross-check vs playwright-core ${playwrightVersion}: ${crossCheck.matched}/${crossCheck.total} match\n`);
}
if (!cleanupOk || crossCheck?.mismatches.length)
  process.exitCode = 1;
