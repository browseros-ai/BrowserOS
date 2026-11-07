// What JSON makes of a run's page, its main frame, its context and a locator, through Playwright
// and through Neo: Playwright's `ChannelOwner.toJSON` gives the first three their type and guid
// only, and a locator its fields (its frame that way, its selector and its `_apiName` tag),
// whether a script stringifies one or returns it. Guids differ by design (Neo's name the tab,
// `page@<targetId>`), so only their type is compared.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).
import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  compareMethod,
  connectCdp,
  launchBrowser,
  openTab,
  playwrightVersion,
  runCleanups,
  runInNeo,
  runInPlaywright,
  serveFixture,
  skip,
} from './harness.mjs'

const html = readFileSync(new URL('../pages/tab.html', import.meta.url), 'utf8')

/** `value` as JSON, each `_guid` replaced by its type. */
const shape = (value) =>
  JSON.parse(JSON.stringify(value), (key, field) =>
    key === '_guid' ? typeof field : field,
  )

const EXPECTED = {
  page: { _type: 'Page', _guid: 'string' },
  frame: { _type: 'Frame', _guid: 'string' },
  context: { _type: 'BrowserContext', _guid: 'string' },
  locator: {
    _frame: { _type: 'Frame', _guid: 'string' },
    _selector: '#count',
    _apiName: 'Locator',
  },
}

test('JSON.stringify gives a page, its frame and its context their type and guid, and a locator its fields', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `
      const shape = (value) => {
        const json = JSON.stringify(value)
        const parsed = JSON.parse(json)
        return { keys: Object.keys(parsed), type: parsed._type, guid: typeof parsed._guid }
      }
      const locator = JSON.parse(JSON.stringify(page.locator('#count')), (key, field) =>
        key === '_guid' ? typeof field : field)
      return {
        page: shape(page),
        frame: shape(page.mainFrame()),
        context: shape(page.context()),
        locator: { keys: Object.keys(locator), json: locator },
      }
    `,
    probe: 'document.title',
  })
  const keys = ['_type', '_guid']
  assert.deepEqual(outcome.value, {
    page: { keys, type: 'Page', guid: 'string' },
    frame: { keys, type: 'Frame', guid: 'string' },
    context: { keys, type: 'BrowserContext', guid: 'string' },
    locator: {
      keys: ['_frame', '_selector', '_apiName'],
      json: EXPECTED.locator,
    },
  })
})

test(`a script that returns its page, frame, context and a locator returns their JSON, as with Playwright ${playwrightVersion}`, {
  skip,
  timeout: 120_000,
}, async () => {
  const source =
    "return { page, frame: page.mainFrame(), context, locator: page.locator('#count') }"
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: { '/run.html': html },
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const playwright = await chromium.connectOverCDP(
      `http://127.0.0.1:${browser.port}`,
    )
    cleanups.push(() => playwright.close())
    const [context] = playwright.contexts()
    const scripts = mkdtempSync(join(tmpdir(), 'neo-to-json-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const scriptPath = join(scripts, 'returns.js')
    writeFileSync(scriptPath, source)

    const reference = await runInPlaywright(
      context,
      (await openTab(cdp, `${origin}/run.html`)).targetId,
      source,
    )
    const neo = await runInNeo(
      neoRun,
      browser.port,
      (await openTab(cdp, `${origin}/run.html`)).targetId,
      scriptPath,
    )

    assert.equal(reference.error, undefined, JSON.stringify(reference.error))
    assert.deepEqual(shape(reference.value), EXPECTED, 'Playwright')
    assert.equal(neo.error, undefined, JSON.stringify(neo.outcome))
    assert.deepEqual(shape(neo.value), shape(reference.value), 'Neo')
  } finally {
    await runCleanups(cleanups)
  }
})
