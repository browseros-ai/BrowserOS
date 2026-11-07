// Page.addLocatorHandler and Page.removeLocatorHandler through Neo and Playwright on the pinned
// interstitial page (tests/pages/handle-locator.html): an action that finds the handler's
// locator visible runs the handler first (with that same locator), then waits for the locator to
// hide unless `noWaitAfter`, at most `times` times; a wait for a selector runs handlers too;
// `force` skips them; a removed handler no longer runs; a child-frame locator is refused.
// Handler counts, the page's click count, the interstitial's visibility and each rejection's
// kind are compared, never the wording.
//
// From Playwright's specs at the pin (tests/page/page-add-locator-handler.spec.ts at 26137face):
// 'LH-001 should work', 'LH-003 should work with locator.hover()' (Playwright skips it headed; the
// parity browser is headless), 'LH-004 should not work with force:true', 'LH-005 should throw when
// handler times out', 'LH-007 should work with locator.waitFor' (with page.waitForSelector, which
// runs the same pre-checks), 'LH-009 should work when owner frame detaches', 'LH-010 should work
// with times: option', 'LH-011 should wait for hidden by default', 'LH-012 should wait for hidden
// by default 2', 'LH-013 should work with noWaitAfter' and 'LH-014 should removeLocatorHandler'.
// The locator snapshot case follows Playwright's loop A, whose attempts run the same pre-checks
// (server/frames.ts _retryWithProgressIfNotConnected).
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html = readFileSync(
  new URL('../pages/handle-locator.html', import.meta.url),
  'utf8',
)

/** Shows the interstitial the pinned way and resets the page's click count. */
const SETUP = `const setup = (...args) => page.evaluate((args) => {
  window.clicked = 0
  window.setupAnnoyingInterstitial(...args)
}, args)
const facts = async () => ({
  clicked: await page.evaluate(() => window.clicked),
  visible: await page.locator('#interstitial').isVisible(),
})
`

test('a handler runs before the action that finds its locator visible (LH-001)', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${SETUP}
      const counts = { before: 0, after: 0, same: true }
      const original = page.getByText('This interstitial covers the button')
      await page.addLocatorHandler(original, async (argument) => {
        counts.same &&= argument === original
        ++counts.before
        await page.locator('#close').click()
        ++counts.after
      })
      const steps = []
      for (const args of [['mouseover', 1], ['mouseover', 2], ['pointerover', 1, 'capture'], ['none', 1], ['remove', 1], ['hide', 1]]) {
        await page.locator('#aside').hover()
        counts.before = 0
        counts.after = 0
        await setup(...args)
        await page.locator('#target').click()
        steps.push({ before: counts.before, after: counts.after, ...(await facts()) })
      }
      return { steps, same: counts.same }
    `,
    probe: 'window.clicked',
  })
  assert.deepEqual(outcome.value, {
    steps: [1, 2, 1, 1, 1, 1].map((times) => ({
      before: times,
      after: times,
      clicked: 1,
      visible: false,
    })),
    same: true,
  })
})

/** One pinned case on a fresh page: `source` returns its facts, which must be `expected`. */
const pinned = (name, source, expected) =>
  test(name, { skip, timeout: 180_000 }, async (t) => {
    const outcome = await compareMethod(t, {
      html,
      source: `${SETUP}\n${source}`,
      probe: 'window.clicked',
    })
    assert.deepEqual(outcome.value, expected)
  })

pinned(
  'force: true runs no handler (LH-004)',
  `let forced = 0
  await page.addLocatorHandler(page.getByText('This interstitial covers the button'), async () => { ++forced; await page.locator('#close').click() })
  await page.locator('#aside').hover()
  await page.evaluate(() => window.setupAnnoyingInterstitial('none', 1))
  await page.locator('#target').click({ force: true, timeout: 2000 })
  return { forced, visible: await page.locator('#interstitial').isVisible(), clicked: typeof (await page.evaluate(() => window.clicked)) }`,
  // A run's value crosses as JSON, which keeps no `undefined` member: its type does.
  { forced: 0, visible: true, clicked: 'undefined' },
)

pinned(
  'locator.hover runs the handler first (LH-003; the parity browser is headless)',
  `await page.addLocatorHandler(page.getByText('This interstitial covers the button'), async () => { await page.locator('#close').click() })
  await page.locator('#aside').hover()
  await page.evaluate(() => window.setupAnnoyingInterstitial('pointerover', 1, 'capture'))
  await page.locator('#target').hover()
  return {
    visible: await page.locator('#interstitial').isVisible(),
    background: await page.$eval('#target', (e) => window.getComputedStyle(e).backgroundColor),
  }`,
  { visible: false, background: 'rgb(255, 255, 0)' },
)

pinned(
  "a locator snapshot runs the handler first, as Playwright's loop A pre-checks do",
  `let handled = 0
  await page.addLocatorHandler(page.getByText('This interstitial covers the button'), async () => { handled++; await page.locator('#close').click() })
  await page.evaluate(() => window.setupAnnoyingInterstitial('none', 1))
  const snapshot = await page.locator('#target').ariaSnapshot()
  return { handled, visible: await page.locator('#interstitial').isVisible(), snapshot }`,
  { handled: 1, visible: false, snapshot: '- button "Click me"' },
)

pinned(
  'locator.waitFor and page.waitForSelector run the handler first (LH-007)',
  `let handled = 0
  await page.addLocatorHandler(page.getByText('This interstitial covers the button'), async () => { handled++; await page.locator('#close').click() })
  await page.locator('#aside').hover()
  await page.evaluate(() => window.setupAnnoyingInterstitial('remove', 1))
  await page.locator('#target').waitFor()
  const waited = { handled, visible: await page.locator('#interstitial').isVisible() }
  await page.evaluate(() => window.setupAnnoyingInterstitial('remove', 1))
  const handle = await page.waitForSelector('#target')
  const selected = { handled, visible: await page.locator('#interstitial').isVisible(), id: await handle.evaluate((e) => e.id) }
  return { waited, selected }`,
  {
    waited: { handled: 1, visible: false },
    selected: { handled: 2, visible: false, id: 'target' },
  },
)

pinned(
  'a handler that never resolves times the action out and is not entered again (LH-005)',
  `let called = 0
  let release
  const released = new Promise((resolve) => { release = resolve })
  let finish
  const finished = new Promise((resolve) => { finish = resolve })
  await page.addLocatorHandler(page.getByText('This interstitial covers the button'), async () => {
    ++called
    await released
    finish(await page.locator('#close').click({ timeout: 3000 }).then(() => 'clicked', errorCategory))
  })
  await page.locator('#aside').hover()
  await setup('mouseover', 1)
  const first = await page.locator('#target').click({ timeout: 3000 }).then(() => 'resolved', errorCategory)
  const second = await page.locator('#target').click({ timeout: 3000 }).then(() => 'resolved', errorCategory)
  const calledWhileRunning = called
  release()
  return { first, second, calledWhileRunning, finished: await finished }`,
  {
    first: 'TimeoutError',
    second: 'TimeoutError',
    calledWhileRunning: 1,
    finished: 'clicked',
  },
)

pinned(
  'times runs a handler at most that often, noWaitAfter goes on at once (LH-010)',
  `let times = 0
  await page.addLocatorHandler(page.locator('body'), async () => { ++times }, { noWaitAfter: true, times: 2 })
  await page.locator('#aside').hover()
  await setup('mouseover', 4)
  const error = await page.locator('#target').click({ timeout: 3000 }).then(() => 'resolved', errorCategory)
  return { times, error, ...(await facts()) }`,
  { times: 2, error: 'TimeoutError', clicked: 0, visible: true },
)

pinned(
  'the action waits for the handled locator to hide (LH-011)',
  `let hidden = 0
  await page.addLocatorHandler(page.getByRole('button', { name: 'close' }), async (button) => { hidden++; await button.click() })
  await page.locator('#aside').hover()
  await setup('timeout', 1)
  await page.locator('#target').click()
  return { hidden, ...(await facts()) }`,
  { hidden: 1, clicked: 1, visible: false },
)

pinned(
  'a handler that leaves its locator visible times the action out (LH-012)',
  `let stays = 0
  await page.addLocatorHandler(page.getByRole('button', { name: 'close' }), async () => { stays++ })
  await page.locator('#aside').hover()
  await setup('hide', 1)
  const error = await page.locator('#target').click({ timeout: 3000 }).then(() => 'resolved', errorCategory)
  return { stays, error, ...(await facts()) }`,
  { stays: 1, error: 'TimeoutError', clicked: 0, visible: true },
)

pinned(
  'noWaitAfter goes on at once and the next attempt runs the handler again (LH-013)',
  `let noWait = 0
  await page.addLocatorHandler(page.getByRole('button', { name: 'close' }), async (button) => {
    noWait++
    if (noWait === 1) await button.click()
    else await page.locator('#interstitial').waitFor({ state: 'hidden' })
  }, { noWaitAfter: true })
  await page.locator('#aside').hover()
  await setup('timeout', 1)
  await page.locator('#target').click()
  return { noWait, ...(await facts()) }`,
  { noWait: 2, clicked: 1, visible: false },
)

test('a handler whose locator is inside a frame its click removes (LH-009)', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: '<!doctype html><html><body></body></html>',
    source: `
      await page.evaluate(() => {
        const iframe = document.createElement('iframe')
        iframe.src = 'data:text/html,<body>hello from iframe</body>'
        document.body.append(iframe)
        const target = document.createElement('button')
        target.textContent = 'Click me'
        target.id = 'target'
        target.addEventListener('click', () => { window._clicked = true })
        document.body.appendChild(target)
        const close = document.createElement('button')
        close.textContent = 'close'
        close.id = 'close'
        close.addEventListener('click', () => iframe.remove())
        document.body.appendChild(close)
      })
      await page.frameLocator('iframe').locator('body').waitFor()
      let called = 0
      await page.addLocatorHandler(page.frameLocator('iframe').locator('body'), async () => {
        ++called
        await page.locator('#close').click()
      })
      await page.locator('#target').click()
      return { called, frames: await page.locator('iframe').count(), clicked: await page.evaluate(() => window._clicked) }
    `,
    probe: 'window._clicked',
  })
  assert.deepEqual(outcome.value, { called: 1, frames: 0, clicked: true })
})

test('a removed handler no longer runs, and a child-frame locator is refused (LH-014)', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: `${html}<iframe name="child" srcdoc="<button>Inner</button>"></iframe>`,
    source: `${SETUP}
      let called = 0
      await page.addLocatorHandler(page.getByRole('button', { name: 'close' }), async (locator) => { ++called; await locator.click() })
      await setup('hide', 1)
      await page.locator('#target').click()
      const handled = { called, ...(await facts()) }
      await setup('hide', 1)
      await page.removeLocatorHandler(page.getByRole('button', { name: 'close' }))
      const removedError = await page.locator('#target').click({ timeout: 3000 }).then(() => 'resolved', errorCategory)
      const removed = { called, error: removedError, ...(await facts()) }
      const childFrame = await page.addLocatorHandler(page.frame('child').locator('button'), () => {}).then(() => 'resolved', errorCategory)
      const zero = await page.addLocatorHandler(page.locator('#close'), () => {}, { times: 0 }).then((value) => typeof value, errorCategory)
      return { handled, removed, childFrame, zero }
    `,
    probe: 'window.clicked',
  })
  assert.deepEqual(outcome.value, {
    handled: { called: 1, clicked: 1, visible: false },
    removed: { called: 1, error: 'TimeoutError', clicked: 0, visible: true },
    childFrame: 'Error',
    zero: 'undefined',
  })
})
