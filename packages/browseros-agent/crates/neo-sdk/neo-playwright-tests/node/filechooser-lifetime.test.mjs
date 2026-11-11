// biome-ignore-all lint/suspicious/noUndeclaredEnvVars: Standalone native browser regression.
/** Adopted tabs need borrowed interception; detached chooser inputs must survive browser GC. */
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { Cdp } from '../js/browser.mjs'
import { launchBrowser } from './browser.mjs'
import { openClaw } from './claw-loader.mjs'

test('adopted-tab chooser waits/listeners and detached inputs survive their event operation', {
  skip: !process.env.NEO_E2E_BROWSER,
  timeout: 120_000,
}, async () => {
  const output = fs.mkdtempSync(
    path.join(
      process.env.NEO_NODE_OUTPUT_DIR || os.tmpdir(),
      'filechooser-lifetime-',
    ),
  )
  const browser = await launchBrowser(process.env.NEO_E2E_BROWSER, output)
  let sdk
  let cdp
  try {
    sdk = await openClaw(browser.port, {
      stateDir: path.join(output, 'state'),
      timeoutMs: 60_000,
    })
    // The launch tab predates this core connection, so it has no agent creation receipt.
    const [page] = sdk.context.pages()
    assert.ok(page, 'the existing launch tab is available to the SDK')
    const version = await (
      await fetch(`http://127.0.0.1:${browser.port}/json/version`)
    ).json()
    cdp = await Cdp.connect(version.webSocketDebuggerUrl)
    const { targetInfos } = await cdp.send('Target.getTargets')
    const target = targetInfos.find((target) => target.type === 'page')
    assert.ok(target)
    const { sessionId } = await cdp.send('Target.attachToTarget', {
      targetId: target.targetId,
      flatten: true,
    })

    await page.setContent('<input type=file>')
    const waiting = page.waitForEvent('filechooser', { timeout: 10_000 })
    await page.click('input')
    const chooser = await waiting
    assert.equal(chooser.page(), page)
    await chooser.setFiles({
      name: 'chosen.txt',
      mimeType: 'text/plain',
      buffer: sdk.Buffer.from('chosen'),
    })
    assert.equal(
      await page.$eval('input', (input) => input.files[0].name),
      'chosen.txt',
    )

    const detached = page.waitForEvent('filechooser', { timeout: 10_000 })
    await page.evaluate(() => {
      const input = document.createElement('input')
      input.type = 'file'
      input.click()
    })
    const temporary = await detached
    await cdp.send('HeapProfiler.collectGarbage', {}, sessionId)
    await temporary.setFiles({
      name: 'retained.txt',
      mimeType: 'text/plain',
      buffer: sdk.Buffer.from('retained'),
    })
    assert.equal(
      await temporary.element().evaluate((input) => input.files[0].name),
      'retained.txt',
    )

    let listener
    const listened = new Promise((resolve) => {
      listener = resolve
    })
    page.on('filechooser', listener)
    await page.click('input')
    assert.equal((await listened).page(), page)
    page.off('filechooser', listener)
    // This write waits for off's native restoration, rather than leaving teardown to race it.
    await page.setContent('<p>finished</p>')
  } finally {
    try {
      await sdk?.finish()
    } finally {
      try {
        await cdp?.close()
      } finally {
        await browser.close()
      }
    }
  }
})
