// Correctness check: completed runs must release their main-world InjectedScript listeners.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  connectCdp,
  launchBrowser,
  openTab,
  runCleanups,
  serveFixture,
  skip,
} from './harness.mjs'

test('main-world injected listeners are released after completed runs', {
  skip,
  timeout: 180000,
}, async (t) => {
  const cleanups = [],
    dir = mkdtempSync(join(tmpdir(), 'neo-main-injected-lifecycle-'))
  cleanups.push(() => rmSync(dir, { recursive: true, force: true }))
  try {
    const bin = buildNeoRun(),
      origin = await serveFixture(cleanups, {
        pages: {
          '/case':
            '<!doctype html><button id="lifecycle-button">button</button>',
        },
      })
    const browser = await launchBrowser(cleanups, browserBinary),
      cdp = await connectCdp(cleanups, browser.port)
    const { targetId, sessionId } = await openTab(cdp, origin + '/case')
    const inspect = async () => {
      const r = await cdp.send(
        'Runtime.evaluate',
        {
          expression: `({globals:Object.getOwnPropertyNames(globalThis),check:(getEventListeners(window).__playwright_global_listeners_check__??[]).length,click:(getEventListeners(window).click??[]).length})`,
          includeCommandLineAPI: true,
          returnByValue: true,
        },
        sessionId,
      )
      assert.equal(
        r.exceptionDetails,
        undefined,
        JSON.stringify(r.exceptionDetails),
      )
      return r.result.value
    }
    const before = await inspect(),
      samples = []
    for (let i = 0; i < 3; i++) {
      const path = join(dir, `run-${i}.js`)
      writeFileSync(
        path,
        `await (await page.$('#lifecycle-button')).dispatchEvent('click');return true`,
      )
      const child = spawn(bin, [
        '--cdp-port',
        String(browser.port),
        '--page',
        targetId,
        '--timeout-ms',
        '30000',
        path,
      ])
      let out = '',
        err = ''
      child.stdout.on('data', (d) => (out += d))
      child.stderr.on('data', (d) => (err += d))
      await once(child, 'exit')
      const outcome = JSON.parse(out)
      assert.equal(outcome.status, 'ok', JSON.stringify(outcome) + ' ' + err)
      const after = await inspect()
      samples.push({
        check: after.check - before.check,
        click: after.click - before.click,
        newGlobals: after.globals.filter(
          (name) => !before.globals.includes(name),
        ),
      })
    }
    t.diagnostic(
      JSON.stringify({
        before: { check: before.check, click: before.click },
        samples,
      }),
    )
    assert.deepEqual(
      samples,
      [0, 1, 2].map(() => ({ check: 0, click: 0, newGlobals: [] })),
      'the completed run leaves no injected window listeners behind',
    )
  } finally {
    await runCleanups(cleanups)
  }
})
