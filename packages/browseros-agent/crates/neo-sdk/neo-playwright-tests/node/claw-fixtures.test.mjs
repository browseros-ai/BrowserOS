/** Real v1 objects, explicit context options, builtins, and isolation across consecutive tests. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { agent, source } from '../js/prepare.mjs'
import { cleanupWorkers } from './browser.mjs'
import { materialize } from './overlay.mjs'
import { classifyRuns, flattenResults } from './report/classify.mjs'

test('tests that real claw fixtures preserve objects, options, builtins and storage isolation', {
  skip:
    !process.env.NEO_E2E_BROWSER ||
    !fs.existsSync(new URL('./claw-addon/claw_node.node', import.meta.url)),
  timeout: 180_000,
}, async () => {
  const root =
    process.env.NEO_NODE_OUTPUT_DIR ||
    path.resolve(agent, '../../.llm/claw-node-results')
  fs.mkdirSync(root, { recursive: true })
  const output = fs.realpathSync(
    fs.mkdtempSync(path.join(root, 'claw-fixture-probe-')),
  )
  const overlay = path.join(output, 'upstream')
  const { configPath } = materialize(overlay, {
    engine: 'claw',
    reference: false,
    output,
    workers: 1,
    curation: false,
  })
  const { default: config } = await import(pathToFileURL(configPath).href)
  config.testMatch = ['claw-fixture.spec.ts']
  fs.writeFileSync(configPath, `export default ${JSON.stringify(config)};\n`)
  fs.writeFileSync(
    path.join(overlay, 'tests/claw-fixture.spec.ts'),
    `
import fs from 'node:fs';
import path from 'node:path';
import { test, expect } from ${JSON.stringify(fileURLToPath(new URL('./fixtures.mjs', import.meta.url)))};
test('real identities and default options', async ({ page, context, browser, playwright, server, httpsServer, __neoWorker }) => {
  expect(page.context()).toBe(context);
  expect(context.browser()).toBe(browser);
  expect(browser.contexts()).toEqual([context]);
  expect(Object.keys(playwright)).toEqual([]);
  await page.goto(server.EMPTY_PAGE);
  const defaults = await page.evaluate(() => ({ viewport: { width: innerWidth, height: innerHeight }, deviceScaleFactor: devicePixelRatio, locale: navigator.language, offline: !navigator.onLine, hasTouch: navigator.maxTouchPoints > 0, isMobile: /Mobile/.test(navigator.userAgent), javaScriptEnabled: true, colorScheme: matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light', contrast: matchMedia('(prefers-contrast: no-preference)').matches ? 'no-preference' : 'other', forcedColors: matchMedia('(forced-colors: active)').matches ? 'active' : 'none', reducedMotion: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'reduce' : 'no-preference' }));
  expect(defaults).toEqual({ viewport: { width: 1280, height: 720 }, deviceScaleFactor: 1, locale: 'en-US', offline: false, hasTouch: false, isMobile: false, javaScriptEnabled: true, colorScheme: 'light', contrast: 'no-preference', forcedColors: 'none', reducedMotion: 'no-preference' });
  server.setRoute('/claw-csp', (_, res) => { res.setHeader('Content-Security-Policy', "script-src 'none'"); res.end('<script>window.inlineRan = true</script>'); });
  await page.goto(server.PREFIX + '/claw-csp');
  expect(await page.evaluate(() => !!window.inlineRan)).toBe(false);
  defaults.bypassCSP = false;
  await expect(page.goto(httpsServer.EMPTY_PAGE)).rejects.toThrow();
  defaults.ignoreHTTPSErrors = false;
  fs.writeFileSync(path.join(__neoWorker.output, 'defaults.json'), JSON.stringify(defaults, null, 2));
});
test('seed storage and fixture builtins', async ({ page, context, server }) => {
  server.setRoute('/claw-sw.js', (_, res) => { res.setHeader('Content-Type', 'text/javascript'); res.end('self.addEventListener("install", () => self.skipWaiting());'); });
  await page.goto(server.EMPTY_PAGE);
  expect(await page.evaluate(() => typeof window.builtins.setTimeout)).toBe('function');
  await page.evaluate(async () => {
    document.cookie = 'claw=value'; localStorage.setItem('claw', 'value'); sessionStorage.setItem('claw', 'value');
    await new Promise((resolve, reject) => { const request = indexedDB.open('claw-db'); request.onsuccess = () => { request.result.close(); resolve(); }; request.onerror = reject; });
    await caches.open('claw-cache');
    await navigator.serviceWorker.register('/claw-sw.js');
    await navigator.serviceWorker.ready;
  });
  context.on('claw-sentinel', () => {});
  const ownPage = await context.newPage();
  expect(await ownPage.evaluate(() => typeof window.builtins)).toBe('undefined');
});
test('next test starts with cleared storage and listeners', async ({ page, context, server }) => {
  await page.goto(server.EMPTY_PAGE);
  expect(await page.evaluate(async () => ({ cookie: document.cookie, local: localStorage.length, session: sessionStorage.length, databases: (await indexedDB.databases()).length, caches: (await caches.keys()).length, workers: (await navigator.serviceWorker.getRegistrations()).length }))).toEqual({ cookie: '', local: 0, session: 0, databases: 0, caches: 0, workers: 0 });
  expect(context.listenerCount('claw-sentinel')).toBe(0);
  expect(context.pages().length).toBe(2);
});
test('the first context factory works, the second is really refused', async ({ contextFactory }) => {
  const context = await contextFactory();
  expect(context).toBeTruthy();
  await expect(contextFactory()).rejects.toThrow('Neo has one browser profile');
});
test('explicit requested default is really refused', async ({ contextFactory }) => {
  await expect(contextFactory({ viewport: { width: 1280, height: 720 } })).rejects.toThrow('Neo has one browser profile');
});
test('expectLoop uses real v1 constructors', async ({ page }) => {
  await page.goto('data:text/html,<h1>hello</h1>');
  await expect(page.locator('h1')).toHaveText('hello');
});
for (const [name, options] of [ ['viewport', { viewport: { width: 1280, height: 720 } }], ['offline', { offline: false }], ['contextOptions', { contextOptions: { acceptDownloads: true } }] ]) test.describe(name, () => {
  test.use(options);
  test('explicit ' + name, async ({ context }) => {});
});
`,
  )
  const fd = fs.openSync(path.join(output, 'probe.log'), 'wx')
  const child = spawn(
    process.execPath,
    [
      path.join(source, 'packages/playwright/cli.js'),
      'test',
      '--config',
      configPath,
    ],
    {
      cwd: overlay,
      stdio: ['ignore', fd, fd],
      env: {
        ...process.env,
        NEO_NODE_MODE: 'claw',
        NEO_NODE_OVERLAY: overlay,
        NEO_NODE_OUTPUT: output,
        NEO_NODE_HOSTED_MODE: '0',
        NEO_NODE_EXPECT_LOOP: '1',
        NEO_NODE_CURATION: '',
        NEO_NODE_LAUNCHER_PID: String(process.pid),
        PWTEST_UNDER_TEST: '1',
      },
    },
  )
  fs.closeSync(fd)
  try {
    await new Promise((resolve, reject) => {
      child.once('exit', resolve)
      child.once('error', reject)
    })
  } finally {
    await cleanupWorkers(output)
  }
  const results = JSON.parse(fs.readFileSync(path.join(output, 'results.json')))
  for (const row of flattenResults(results)) {
    assert.equal(
      row.test.status,
      row.title.includes('explicit viewport') ||
        row.title.includes('explicit offline') ||
        row.title.includes('explicit contextOptions')
        ? 'unexpected'
        : 'expected',
      `${row.title}: ${output}/probe.log`,
    )
  }
  const rows = classifyRuns(results, { suites: [] }, [], { engine: 'claw' })
  assert.equal(rows.filter((row) => row.verdict === 'neo-lacks').length, 3)
  assert.equal(
    rows.find((row) => row.title.includes('expectLoop uses')).verdict,
    'pass via expectLoop',
  )
  for (const row of flattenResults(results).filter(
    (row) => row.title.includes('explicit ') || row.title.includes('second is'),
  ))
    assert.ok(
      row.test.annotations.some(
        (annotation) => annotation.type === 'claw-context',
      ),
      row.title,
    )
  console.log(`Claw fixture evidence: ${output}`)
})
