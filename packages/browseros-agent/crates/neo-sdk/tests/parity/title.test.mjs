// A title read has no action timeout, on fresh pages serving the existing dialog fixture.
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
  waitForUrl,
} from './harness.mjs'

test('title returns the same string with a tiny action timeout', {
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
    const page = await browser.contexts()[0].newPage()
    await page.goto(`${origin}/dialog`)
    const source =
      'page.setDefaultTimeout(1); const title=await page.title(); return {title,type:typeof title}'
    const reference = await new (async () => {}).constructor('page', source)(
      page,
    )
    const { targetId } = await cdp.send('Target.createTarget', {
      url: `${origin}/dialog`,
    })
    cleanups.push(() => cdp.send('Target.closeTarget', { targetId }))
    const { sessionId } = await cdp.send('Target.attachToTarget', {
      targetId,
      flatten: true,
    })
    await waitForUrl(cdp, sessionId, `${origin}/dialog`)
    const scripts = mkdtempSync(join(tmpdir(), 'neo-title-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const script = join(scripts, 'title.js')
    writeFileSync(script, source)
    const outcome = await startNeo(binary, port, targetId, script).outcome
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
    assert.deepEqual(outcome.value, reference)
    assert.deepEqual(reference, { title: 'dialog', type: 'string' })
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
      await page.locator('button').textContent({ timeout: 5000 }),
    )
  } finally {
    await runCleanups(cleanups)
  }
})
