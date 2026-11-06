// textContent through the Playwright reference and through Neo, on fresh pages of the dialog
// fixture: the values and their types (a strict locator read, a non-strict page read whose first
// match is empty), what the read saw in the DOM, and three rejections compared by error name only
// (a strict mode violation, a timeout, a selector that is not a string).
//
// `body > button`, not `button`: BrowserOS neo's takeover bar (claw-app's takeover content
// script, an open shadow root anchored to the body at document_idle) adds three buttons to every
// page, and Playwright's CSS engine pierces open shadow roots.
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
  const rejected = (e) => ({ rejected: true, name: e.name })
  const button = await page.locator('body > button').textContent()
  const first = await page.textContent('head > *')
  return {
    button,
    buttonType: typeof button,
    first,
    firstType: typeof first,
    strict: await page.locator('head > *').textContent().catch(rejected),
    missing: await page.locator('#missing').textContent({ timeout: 300 }).catch(rejected),
    notAString: await page.textContent(42).catch(rejected),
  }
`

test('textContent returns the same values and rejects the same calls', {
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

    const { targetId } = await cdp.send('Target.createTarget', {
      url: `${origin}/dialog`,
    })
    cleanups.push(() => cdp.send('Target.closeTarget', { targetId }))
    const { sessionId } = await cdp.send('Target.attachToTarget', {
      targetId,
      flatten: true,
    })
    await waitForUrl(cdp, sessionId, `${origin}/dialog`)
    const scripts = mkdtempSync(join(tmpdir(), 'neo-text-content-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const script = join(scripts, 'text-content.js')
    writeFileSync(script, source)
    const outcome = await startNeo(binary, port, targetId, script).outcome

    assert.deepEqual(reference, {
      button: 'Submit',
      buttonType: 'string',
      first: '',
      firstType: 'string',
      strict: { rejected: true, name: 'Error' },
      missing: { rejected: true, name: 'TimeoutError' },
      notAString: { rejected: true, name: 'Error' },
    })
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
    assert.deepEqual(outcome.value, reference, 'Neo vs Playwright')
    const visible = await cdp.send(
      'Runtime.evaluate',
      {
        expression: 'document.querySelector("body > button").textContent',
        returnByValue: true,
      },
      sessionId,
    )
    assert.equal(outcome.value.button, visible.result.value, 'the DOM agrees')
  } finally {
    await runCleanups(cleanups)
  }
})
