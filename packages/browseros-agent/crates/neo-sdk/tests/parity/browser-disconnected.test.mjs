// The browser's disconnection through Neo and through Playwright on one BrowserOS neo: closing
// the run's own context disconnects the browser as `browser.close()` does, since a connectOverCDP
// default context closes by closing its browser (`CRBrowserContext.doClose`). Each case logs what
// the script sees, in order: every page's `close`, then its context's `pageclose`, each created
// context's `close`, the run's own, then `disconnected`, with `isConnected()` read in each
// listener; then how the waits it interrupted reject and how later calls fail. The run's tab
// stays open in BrowserOS, and the context the script created is gone from it.
//
// No Playwright test (pin 26137face) closes a connectOverCDP default context; the order, the
// reasons and the state below are pinned Playwright's, measured here on every run.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { fixturePage, outcomeValue, withPlatform } from './platform.mjs'

const pages = {
  '/emulation.html': fixturePage('emulation.html'),
  '/debugger.html':
    '<title>debugger</title><button onclick="window.clicks++">go</button><script>window.clicks = 0</script>',
}

/** Logs the run's events for the run page and one created page; other tabs are the harness's. */
const LISTEN = `const browser = context.browser()
const log = []
const created = await browser.newContext()
const other = await created.newPage()
const named = (p) => (p === page ? 'runPage' : p === other ? 'createdPage' : null)
const connected = () => browser.isConnected()
browser.on('disconnected', () => log.push(['disconnected', connected(), browser.contexts().length]))
context.on('close', () => log.push(['run close', connected(), context.pages().length]))
created.on('close', () => log.push(['created close', connected(), created.pages().length]))
page.on('close', () => log.push(['runPage close', page.isClosed(), connected()]))
other.on('close', () => log.push(['createdPage close', other.isClosed(), connected()]))
context.on('pageclose', (p) => named(p) && log.push(['run pageclose', named(p)]))
const firstLine = (error) => error.message.split('\\n')[0]
const waits = [other, page].map((p) => p.waitForEvent('framenavigated').then(() => 'resolved', firstLine))
`

/** What every object reads, and how every kind of call fails, once the run disconnected. */
const AFTER = `const reads = { connected: browser.isConnected(), contexts: browser.contexts().length,
  closed: [context.isClosed(), page.isClosed(), created.isClosed(), other.isClosed()],
  pages: [context.pages().length, created.pages().length] }
const fails = await Promise.all([other.title(), page.title(), browser.newContext(), context.newPage()]
  .map((call) => call.then(() => 'resolved', firstLine)))
return { log, waits: await Promise.all(waits), reads, fails }`

const fails = [
  'page.title: Target page, context or browser has been closed',
  'page.title: Target page, context or browser has been closed',
  'browser.newContext: Target page, context or browser has been closed',
  'browserContext.newPage: Target page, context or browser has been closed',
]
const state = {
  connected: false,
  contexts: 0,
  closed: [true, true, true, true],
  pages: [0, 0],
}
const order = (connectedInContexts) => [
  ['runPage close', true, true],
  ['run pageclose', 'runPage'],
  ['createdPage close', true, true],
  ['created close', connectedInContexts, 0],
  ['run close', connectedInContexts, 0],
  ['disconnected', false, 0],
]

const cases = [
  {
    name: "the run's own context closes: the browser disconnects",
    source: `${LISTEN}await context.close({ reason: 'run done' })
${AFTER}`,
    expected: {
      log: order(true),
      // A created page's wait has no reason of its own: its context gave none, nor the browser.
      waits: [
        'page.waitForEvent: Target page, context or browser has been closed',
        'page.waitForEvent: run done',
      ],
      reads: state,
      fails,
    },
  },
  {
    name: 'the browser closes',
    source: `${LISTEN}await browser.close({ reason: 'browser done' })
${AFTER}`,
    expected: {
      log: order(true),
      waits: [
        'page.waitForEvent: browser done',
        'page.waitForEvent: browser done',
      ],
      reads: state,
      fails,
    },
  },
]

for (const { name, source, expected } of cases) {
  test(`browser disconnected: ${name}`, { skip }, async (t) => {
    await withPlatform(t, { pages }, async ({ playwright, neo, cdp }) => {
      const reference = outcomeValue(
        await playwright('/emulation.html', source),
      )
      assert.deepEqual(reference, expected, `${name}: Playwright`)
      const result = await neo('/emulation.html', source)
      assert.deepEqual(outcomeValue(result), reference, `${name}: Neo`)
      const { targetInfos } = await cdp.send('Target.getTargets')
      assert.ok(
        targetInfos.some((target) => target.targetId === result.targetId),
        "the run's tab stays open",
      )
      const { browserContextIds } = await cdp.send('Target.getBrowserContexts')
      assert.deepEqual(browserContextIds, [], 'the created context is gone')
    })
  })
}

// Playwright's call its debugger holds waits for a resume that can no longer come, and never
// settles; Neo's rejects as closed, as every call the close interrupted does (a Deviation,
// neo-runtime `dispatch.rs` `released`). Each engine's side is asserted; on both the close
// resolves and the debugger still names the held call.
test('browser disconnected: a call the debugger holds', { skip }, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo, cdp }) => {
    const source = `const dbg = context.debugger
const pending = new Promise((resolve) => setTimeout(() => resolve('pending'), 1000))
await dbg.requestPause()
const clicked = page.click('button').then(() => 'resolved', (error) => error.message.split('\\n')[0])
await new Promise((resolve) => dbg.once('pausedstatechanged', resolve))
const closed = await context.close({ reason: 'run done' }).then(() => 'closed')
return { closed, held: dbg.pausedDetails().title.startsWith('Click'),
  click: await Promise.race([clicked, pending]), connected: context.browser().isConnected() }`
    const both = { closed: 'closed', held: true, connected: false }
    assert.deepEqual(
      outcomeValue(await playwright('/debugger.html', source)),
      { ...both, click: 'pending' },
      'Playwright',
    )
    const result = await neo('/debugger.html', source)
    assert.deepEqual(
      outcomeValue(result),
      { ...both, click: 'page.click: run done' },
      'Neo',
    )
    // The run's tab stays open, and the click the debugger held never reached it.
    const { sessionId } = await cdp.send('Target.attachToTarget', {
      targetId: result.targetId,
      flatten: true,
    })
    const { result: clicks } = await cdp.send(
      'Runtime.evaluate',
      { expression: 'window.clicks', returnByValue: true },
      sessionId,
    )
    assert.equal(clicks.value, 0)
  })
})
