// context.debugger through Neo and through Playwright on one BrowserOS neo: a requested pause
// holds the next pausable call before it runs (its click has not reached the page while held),
// `pausedDetails()` names it until `resume()` lets it go, `page.pause()` stops there too once the
// debugger is enabled, a created context's debugger holds only that context's calls, and
// resuming a debugger that holds nothing fails.
//
// Playwright cases (pin 26137face): `tests/library/debugger.spec.ts` "DB-001 should pause at
// next and resume" and "DB-002 should pause at pause call"; the not-paused errors are
// `server/debugger.ts` `Debugger.doResume` and `Debugger.next`. The held call's location names
// each engine's own script file, so only its shape is compared.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const pages = {
  '/debugger.html':
    '<title>debugger</title><button onclick="document.getElementById(\'count\').textContent = ++window.clicks">go</button><p id="count">0</p><script>window.clicks = 0</script>',
}

const changed = `const changed = (dbg) => new Promise((resolve) => dbg.once('pausedstatechanged', resolve))
const count = () => page.locator('#count').textContent()
`

const cases = [
  {
    name: 'pauses at next and resumes',
    source: `${changed}const dbg = context.debugger
const before = dbg.pausedDetails()
await dbg.requestPause()
const clicked = page.click('button')
await changed(dbg)
const held = dbg.pausedDetails()
const whileHeld = await count()
await Promise.all([dbg.resume(), changed(dbg), clicked])
return {
  before,
  held: { title: held.title.startsWith('Click'), file: typeof held.location.file, line: typeof held.location.line },
  whileHeld,
  after: dbg.pausedDetails(),
  clicks: await count(),
}`,
    expected: {
      before: null,
      held: { title: true, file: 'string', line: 'number' },
      whileHeld: '0',
      after: null,
      clicks: '1',
    },
  },
  {
    name: 'pauses at a pause call',
    source: `${changed}const dbg = context.debugger
await dbg.requestPause()
const paused = page.pause()
await changed(dbg)
const title = dbg.pausedDetails().title
await dbg.resume()
await paused
return { title, after: dbg.pausedDetails() }`,
    expected: { title: 'Pause', after: null },
  },
  {
    name: "a created context's debugger holds only its own calls",
    source: `${changed}const created = await context.browser().newContext()
await created.debugger.requestPause()
const own = await page.title()
const other = await created.newPage()
const navigated = other.goto(page.url())
await changed(created.debugger)
const held = created.debugger.pausedDetails().title.startsWith('Navigate')
const ownDebugger = context.debugger.pausedDetails()
await created.debugger.resume()
await navigated
const title = await other.title()
await created.close()
return { own, held, ownDebugger, title }`,
    expected: {
      own: 'debugger',
      held: true,
      ownDebugger: null,
      title: 'debugger',
    },
  },
  {
    name: 'resuming a debugger that holds nothing fails',
    source: `const dbg = context.debugger
return [
  await dbg.resume().then(() => 'resumed', errorCategory),
  await dbg.next().then(() => 'stepped', errorCategory),
  await dbg.runTo({ file: 'run.js' }).then(() => 'ran', errorCategory),
]`,
    expected: ['Error', 'Error', 'Error'],
  },
]

test('context.debugger: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    for (const { name, source, expected } of cases) {
      const reference = outcomeValue(await playwright('/debugger.html', source))
      assert.deepEqual(reference, expected, `${name}: Playwright`)
      assert.deepEqual(
        outcomeValue(await neo('/debugger.html', source)),
        reference,
        `${name}: Neo`,
      )
    }
  })
})
