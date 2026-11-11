/** Neo: Exercises the fixture seams with the real SDK, including explicitly requested defaults. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { agent, source } from '../js/prepare.mjs'
import { cleanupWorkers } from './browser.mjs'
import { materialize } from './overlay.mjs'
import { classifyRuns } from './report/classify.mjs'

test('upstream fixtures retain real objects and expose unsupported internals explicitly', {
  skip:
    !process.env.NEO_E2E_BROWSER ||
    !fs.existsSync(
      new URL('../../neo-quickjs-injected/dist/sdk.js', import.meta.url),
    ),
  timeout: 90_000,
}, async () => {
  const root =
    process.env.NEO_NODE_OUTPUT_DIR ||
    path.resolve(agent, '../../.llm/neo-node-results')
  fs.mkdirSync(root, { recursive: true })
  const output = fs.realpathSync(
    fs.mkdtempSync(path.join(root, 'fixture-probe-')),
  )
  const overlay = path.join(output, 'upstream')
  const { configPath } = materialize(overlay, {
    reference: false,
    output,
    workers: 2,
  })
  const { default: config } = await import(pathToFileURL(configPath).href)
  config.testMatch = ['contract', 'trace', 'video', 'expect'].map((name) =>
    path.join(overlay, `tests/fixture-${name}.spec.ts`),
  )
  fs.writeFileSync(
    path.join(overlay, 'tests/fixture-expect.spec.ts'),
    `
import { test, expect, rafraf } from './page/pageTest';
import { expect as browserExpect } from './config/browserTest';
test('expect import seams preserve plain assertions', async ({ page }) => {
  await page.setContent('<h1>hello</h1>');
  await expect(page.locator('h1')).toHaveText('hello');
  await expect(page).toHaveTitle('');
  await browserExpect(page.locator('h1')).toBeVisible();
  await browserExpect(page).toHaveURL('about:blank');
  expect('a: 1\\nb: 2').toContainYaml('  a: 1');
  expect(typeof rafraf).toBe('function');
  await expect.poll(() => 'ready').toBe('ready');
  expect.soft(1).toBe(1);
  expect.configure({ timeout: 50 })('still real').toContain('real');
  expect.extend({ toBeCustom: value => ({ pass: value === 42, message: () => 'custom' }) })(42).toBeCustom();
});
test('uncovered matcher skips immediately', async ({ page }) => {
  await expect(page.locator('body')).toMatchAriaSnapshot('- generic');
});
test('caught uncovered matcher still skips', async ({ page }) => {
  const error = await expect(page.locator('body')).toMatchAriaSnapshot('- generic').catch(e => e);
  expect(error.message).toBe('upstream matcher error text');
});
test('loop assertion failures retain their prefix', async ({ page }) => {
  await expect(page.locator('missing')).toHaveCount(1, { timeout: 10 });
});
`,
  )
  fs.writeFileSync(configPath, `export default ${JSON.stringify(config)};\n`)
  fs.writeFileSync(
    path.join(overlay, 'tests/fixture-contract.spec.ts'),
    `
import fs from 'node:fs';
import path from 'node:path';
import { test, expect } from ${JSON.stringify(fileURLToPath(new URL('./fixtures.mjs', import.meta.url)))};
import { stopBrowser } from ${JSON.stringify(fileURLToPath(new URL('./browser.mjs', import.meta.url)))};
test('implicit defaults and identities', async ({ browser, context, page, playwright, browserVersion, isHeadlessShell, channel, mode }) => {
  expect(page.context()).toBe(context);
  expect(context.browser()).toBe(browser);
  expect(browser.browserType()).toBe(playwright.chromium);
  expect(browserVersion).toMatch(/^\\d+\\./);
  expect(isHeadlessShell).toBe(false);
  expect(channel).toBe('chromium');
  expect(mode).toBe('default');
});
test('contextFactory and pageWithHar use native options', async ({ contextFactory, pageWithHar }) => {
  const context = await contextFactory();
  expect((await context.newPage()).context()).toBe(context);
  await expect(contextFactory({ offline: false })).rejects.toThrow('offline');
  await expect(contextFactory({ acceptDownloads: true })).rejects.toThrow('acceptDownloads');
  await expect(pageWithHar()).rejects.toThrow('recordHar');
});
test('private fixtures fail explicitly', async ({ toImpl, startRemoteServer }) => {
  expect(() => toImpl()).toThrow(/^needs Playwright internals: toImpl/);
  expect(() => startRemoteServer('launchServer')).toThrow(/^needs Playwright internals: startRemoteServer/);
});
test('public launch APIs keep the SDK refusal', async ({ browserType, launchPersistent, createUserDataDir }) => {
  const dir = await createUserDataDir();
  expect(fs.existsSync(dir)).toBe(true);
  await expect(browserType.launch()).rejects.toThrow('Neo runs attached');
  await expect(launchPersistent()).rejects.toThrow('Neo runs attached');
});
test('file paths share the copied working directory', async ({ page }, testInfo) => {
  await page.setContent('<input type="file">');
  await page.setInputFiles('input', 'tests/assets/file-to-upload.txt');
  expect(await page.locator('input').evaluate(el => el.files[0].name)).toBe('file-to-upload.txt');
  const file = testInfo.outputPath('proof.png');
  await page.screenshot({ path: path.relative(process.cwd(), file) });
  expect(fs.existsSync(file)).toBe(true);
});
test('crashed browser fails its worker', async ({ page, __neoWorker }) => {
  await stopBrowser(__neoWorker);
  await page.title();
});
test('next test gets a replacement worker', async ({ page }) => {
  expect(await page.title()).toBe('');
});
for (const [name, options] of [
  ['explicit offline', { offline: false }],
  ['explicit acceptDownloads', { acceptDownloads: true }],
  ['explicit contextOptions', { contextOptions: { offline: false } }],
]) test.describe(name, () => {
  test.use(options);
  test(name, async ({ context }) => {});
});
`,
  )
  for (const option of ['trace', 'video'])
    fs.writeFileSync(
      path.join(overlay, `tests/fixture-${option}.spec.ts`),
      `
import { test } from ${JSON.stringify(fileURLToPath(new URL('./fixtures.mjs', import.meta.url)))};
test.use({ ${option}: 'on' });
test('${option} option', async ({ context }) => {});
`,
    )
  const log = fs.openSync(path.join(output, 'probe.log'), 'wx')
  let child
  try {
    child = spawn(
      process.execPath,
      [
        path.join(source, 'packages/playwright/cli.js'),
        'test',
        '--config',
        configPath,
      ],
      {
        cwd: overlay,
        stdio: ['ignore', log, log],
        env: {
          ...process.env,
          NEO_NODE_OVERLAY: overlay,
          NEO_NODE_OUTPUT: output,
          NEO_NODE_MODE: 'neo',
          NEO_NODE_LAUNCHER_PID: String(process.pid),
          PWTEST_UNDER_TEST: '1',
        },
      },
    )
  } finally {
    fs.closeSync(log)
  }
  try {
    await new Promise((resolve, reject) => {
      child.once('error', reject)
      child.once('exit', resolve)
    })
  } finally {
    await cleanupWorkers(output)
  }
  const report = JSON.parse(fs.readFileSync(path.join(output, 'results.json')))
  const results = new Map()
  function collect(suite) {
    for (const spec of suite.specs || []) results.set(spec.title, spec.tests[0])
    for (const child of suite.suites || []) collect(child)
  }
  collect(report)
  const rows = classifyRuns(report, { suites: [] })
  assert.equal(
    rows.find(
      (row) => row.title === 'expect import seams preserve plain assertions',
    )?.verdict,
    'pass via expectLoop',
  )
  const skipped = rows.find(
    (row) => row.title === 'uncovered matcher skips immediately',
  )
  assert.equal(skipped?.verdict, 'skip')
  assert.equal(
    skipped?.reason,
    'expectLoop: toMatchAriaSnapshot is not covered',
  )
  assert.equal(
    rows.find((row) => row.title === 'caught uncovered matcher still skips')
      ?.verdict,
    'skip',
  )
  const failed = rows.find(
    (row) => row.title === 'loop assertion failures retain their prefix',
  )
  assert.equal(failed?.verdict, 'neo-differs')
  assert.match(failed.evidence[0].message, /expectLoop: toHaveCount/)
  assert.equal(
    results.get('crashed browser fails its worker')?.status,
    'unexpected',
  )
  const recovered = results.get('next test gets a replacement worker')
  assert.equal(recovered?.status, 'expected')
  assert.notEqual(
    recovered.results[0].workerIndex,
    results.get('crashed browser fails its worker').results[0].workerIndex,
  )
  for (const title of [
    'implicit defaults and identities',
    'contextFactory and pageWithHar use native options',
    'private fixtures fail explicitly',
    'public launch APIs keep the SDK refusal',
    'file paths share the copied working directory',
  ])
    assert.equal(
      results.get(title)?.status,
      'expected',
      `${title}: see ${output}/probe.log`,
    )
  for (const [title, message] of [
    ['explicit offline', 'offline'],
    ['explicit acceptDownloads', 'acceptDownloads'],
    ['explicit contextOptions', 'offline'],
    ['trace option', 'needs Playwright internals: trace'],
    ['video option', 'recordVideo'],
  ]) {
    const result = results.get(title)
    assert.equal(result?.status, 'unexpected', title)
    assert.ok(
      result.results
        .flatMap((value) => value.errors)
        .some((error) => error.message.includes(message)),
      `${title}: ${JSON.stringify(result)}`,
    )
  }
  console.log(`Fixture evidence: ${output}`)
})
