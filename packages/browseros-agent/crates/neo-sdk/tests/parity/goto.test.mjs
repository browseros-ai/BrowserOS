// Navigation and the documented Response/null difference against selected Playwright.
import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  connectCdp,
  launchBrowser,
  playwrightVersion,
  runCleanups,
  serveFixture,
  skip,
  startNeo,
} from './harness.mjs'

test('goto preserves navigation and reports the Response/null difference', {
  skip,
  timeout: 120_000,
}, async (t) => {
  t.diagnostic(`Playwright reference: ${playwrightVersion}`)
  const binary = buildNeoRun()
  const cleanups = []
  try {
    const html = readFileSync(
      new URL('../pages/dialog.html', import.meta.url),
      'utf8',
    )
    const origin = await serveFixture(cleanups, { pages: { '/dialog': html } })
    const { port } = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, port)
    const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`)
    cleanups.push(() => browser.close())
    const [context] = browser.contexts()
    const page = await context.newPage()
    const source = `
      const response = await page.goto(${JSON.stringify(`${origin}/dialog`)});
      let invalid;
      try { await page.goto('about:blank', {waitUntil:'bogus'}); }
      catch (e) { invalid = e.name; }
      return {response:response===null?null:{status:response.status(),url:response.url()},responseType:typeof response,title:await page.title(),invalid};
    `
    const reference = await new (async () => {}).constructor('page', source)(
      page,
    )
    const { targetId } = await cdp.send('Target.createTarget', {
      url: 'about:blank',
    })
    cleanups.push(() => cdp.send('Target.closeTarget', { targetId }))
    const { sessionId } = await cdp.send('Target.attachToTarget', {
      targetId,
      flatten: true,
    })
    const scripts = mkdtempSync(join(tmpdir(), 'neo-goto-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const script = join(scripts, 'goto.js')
    writeFileSync(script, source)
    const outcome = await startNeo(binary, port, targetId, script).outcome
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
    assert.deepEqual(reference.response, {
      status: 200,
      url: `${origin}/dialog`,
    })
    assert.equal(reference.responseType, 'object')
    assert.equal(outcome.value.responseType, 'object')
    assert.equal(
      outcome.value.response,
      null,
      'Neo intentionally has no Response objects',
    )
    assert.equal(outcome.value.title, reference.title)
    assert.equal(reference.title, 'dialog')
    assert.equal(reference.invalid, 'Error')
    assert.equal(outcome.value.invalid, 'Error')
    const visible = await cdp.send(
      'Runtime.evaluate',
      {
        expression: 'document.querySelector("button").textContent',
        returnByValue: true,
      },
      sessionId,
    )
    assert.equal(
      visible.result.value,
      await page.locator('button').textContent(),
    )
  } finally {
    await runCleanups(cleanups)
  }
})
