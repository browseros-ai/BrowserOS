import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  launchBrowser,
  playwrightVersion,
  runCleanups,
  serveFixture,
  skip,
  startNeo,
} from './harness.mjs'

const source = `
  const observed = [];
  for (const [family, target] of [['locator', page.locator('#target')], ['page', page], ['frame', page.mainFrame()]]) {
    for (const method of ['isVisible', 'isHidden']) {
      let value, error;
      const options = {signal: {aborted:true}};
      try { value = await target[method](...(family === 'locator' ? [options] : ['#target', options])); }
      catch (caught) { error = caught; }
      observed.push({name: family + '.' + method, value: value ?? null,
        error: error ? error.name : null,
        refused: error instanceof Error && error.name === 'Error' && /\\bsignal\\b/.test(error.message)});
    }
  }
  return observed;
`

test('visibility methods accept signal in the pin but S1 must explicitly refuse it', {
  skip,
  timeout: 180_000,
}, async (t) => {
  t.diagnostic(
    `Playwright ${playwrightVersion}; client/types.ts: TimeoutOptions has signal`,
  )
  const executable = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: { '/visibility': '<button id="target">Target</button>' },
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const playwright = await chromium.connectOverCDP(
      `http://127.0.0.1:${browser.port}`,
    )
    cleanups.push(() => playwright.close())
    const [context] = playwright.contexts()
    const page = await context.newPage()
    cleanups.push(() => page.close())
    await page.goto(`${origin}/visibility`)
    const reference = await new (async () => {}).constructor('page', source)(
      page,
    )
    t.diagnostic('pinned Playwright: ' + JSON.stringify(reference))
    assert.deepEqual(
      reference.map((row) => row.value),
      [true, false, true, false, true, false],
    )
    const session = await context.newCDPSession(page)
    const { targetInfo } = await session.send('Target.getTargetInfo')
    await session.detach()
    const scripts = mkdtempSync(join(tmpdir(), 'neo-s1-visibility-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const file = join(scripts, 'visibility.js')
    writeFileSync(file, source)
    const neo = startNeo(executable, browser.port, targetInfo.targetId, file)
    await neo.connected
    const outcome = await neo.outcome
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
    t.diagnostic('Neo: ' + JSON.stringify(outcome.value))
    t.diagnostic(
      'Neo sent methods: ' +
        JSON.stringify(outcome.calls.map((call) => call.method)),
    )
    assert.deepEqual(
      outcome.value.filter((row) => !row.refused),
      [],
      'DECIDED S1: every accepted signal must be refused',
    )
    assert.deepEqual(
      outcome.calls,
      [],
      'DECIDED S1: refusal must precede browser work',
    )
  } finally {
    await runCleanups(cleanups)
  }
})
