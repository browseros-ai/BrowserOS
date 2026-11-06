// page.ariaSnapshot through the Playwright reference and through Neo, on fresh pages of the dialog
// fixture: the default, ai and depth-limited snapshots as returned strings, an ai ref clicked and
// read through page.locator('aria-ref=…'), the click's effect in the page (the button's confirm()
// was answered, as both dismiss a dialog nobody listens for), and an invalid mode rejected,
// compared by kind only (the error's name), never the wording.
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

const source = `
  const rejected = (e) => ({
    rejected: true,
    name: e.name,
    strict: /strict mode violation/.test(e.message),
  })
  const plain = await page.ariaSnapshot()
  const ai = await page.ariaSnapshot({ mode: 'ai' })
  const shallow = await page.ariaSnapshot({ mode: 'ai', depth: 1 })
  const ref = ai
    .split('\\n')
    .find((line) => line.includes('button "Submit"'))
    ?.match(/\\[ref=(e\\d+)\\]/)?.[1]
  await page.locator('aria-ref=' + ref).click()
  const refText = await page.locator('aria-ref=' + ref).textContent()
  return {
    plain,
    ai,
    shallow,
    type: typeof ai,
    ref,
    refText,
    invalidMode: await page.ariaSnapshot({ mode: 'bogus' }).catch(rejected),
  }
`

test('ariaSnapshot returns the same YAML and its refs click and read', {
  skip,
  timeout: 180_000,
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
    const reference = await new (async () => {}).constructor('page', source)(
      page,
    )
    const referenceAnswer = await page.evaluate(() => window.__answer)

    const { targetId } = await cdp.send('Target.createTarget', {
      url: `${origin}/dialog`,
    })
    cleanups.push(() => cdp.send('Target.closeTarget', { targetId }))
    const { sessionId } = await cdp.send('Target.attachToTarget', {
      targetId,
      flatten: true,
    })
    await waitForUrl(cdp, sessionId, `${origin}/dialog`)
    const scripts = mkdtempSync(join(tmpdir(), 'neo-aria-snapshot-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const script = join(scripts, 'aria-snapshot.js')
    writeFileSync(script, source)
    const outcome = await startNeo(binary, port, targetId, script).outcome
    const answer = await cdp.send(
      'Runtime.evaluate',
      { expression: 'window.__answer', returnByValue: true },
      sessionId,
    )

    assert.deepEqual(reference, {
      plain: '- button "Submit"',
      ai: '- button "Submit" [ref=e2]',
      shallow: '- button "Submit" [ref=e2]',
      type: 'string',
      ref: 'e2',
      refText: 'Submit',
      invalidMode: { rejected: true, name: 'Error', strict: false },
    })
    assert.equal(
      referenceAnswer,
      false,
      'the dismissed confirm() answered false',
    )
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
    assert.deepEqual(outcome.value, reference, 'Neo vs Playwright')
    assert.equal(
      answer.result.value,
      referenceAnswer,
      'the click reached the page',
    )
  } finally {
    await runCleanups(cleanups)
  }
})
