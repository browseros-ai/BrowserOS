// frame.waitForFunction while its iframe changes process, through frames (frames.mjs,
// frame-top.html), Playwright's tab first, then Neo's. A wait started in an iframe keeps waiting
// when the iframe navigates between 127.0.0.1 and localhost, and resolves with what the
// destination document answers. Both directions count: a same-process frame becoming
// out-of-process, and back. Without the swap (cross to cross) it resolves the same way. A frame
// removed under the wait ends it, and an abort still stops it. Values compared as frames.mjs's
// `settle` reports them; errors by category.
//
// Regression for med-pw-int's whole-branch review, finding 2: Neo kept polling the frame's
// retired context and timed out where Playwright resolves. Its fixture,
// pw-int-rev/waitForFunction.test.mjs, is adapted to the shared frame page here.
//
// Cases from tests/page/page-wait-for-function.spec.ts at 26137face: WA-020 "should survive
// cross-process navigation", for an iframe rather than the page; WA-026 "should throw when frame
// is detached".
import { test } from 'node:test'
import { compareFrameCases, failed, value } from './frames.mjs'
import { skip } from './harness.mjs'

const byName = (name) =>
  `page.frames().find((frame) => frame.name() === ${JSON.stringify(name)})`

/** A wait in frame `name`, then that iframe (`selector`) navigated to `host`'s child page. */
const swap = (name, selector, host) => `
  const frame = ${byName(name)}
  const top = page.url()
  const base = top.slice(0, top.indexOf('/frame-top.html'))
  const destination = ${host === 'cross' ? "base.replace('127.0.0.1', 'localhost')" : 'base'}
  const waiting = settle(async () => {
    const handle = await frame.waitForFunction(() => window.__who === 'swapped' && 42, undefined, { timeout: 5000 })
    const answer = await handle.jsonValue()
    await handle.dispose()
    return answer
  })
  await new Promise((resolve) => setTimeout(resolve, 250))
  await page.evaluate((url) => { document.querySelector(${JSON.stringify(selector)}).src = url }, destination + '/frame-child.html?who=swapped')
  return {
    waited: await waiting,
    final: await settle(() => ${byName(name)}.evaluate(() => window.__who)),
  }`

const cases = [
  {
    name: 'a same-process frame that becomes out-of-process keeps its wait',
    source: swap('same', '#same', 'cross'),
    expected: { waited: value(42), final: value('swapped') },
  },
  {
    name: 'an out-of-process frame that comes back in process keeps its wait',
    source: swap('cross', '#login-iframe', 'same'),
    expected: { waited: value(42), final: value('swapped') },
  },
  {
    name: 'an out-of-process frame that stays out of process keeps its wait',
    source: swap('cross', '#login-iframe', 'cross'),
    expected: { waited: value(42), final: value('swapped') },
  },
  {
    name: 'a frame removed under the wait ends it',
    source: `
      const frame = ${byName('same')}
      const waiting = settle(() => frame.waitForFunction(() => false, undefined, { timeout: 5000 }))
      await new Promise((resolve) => setTimeout(resolve, 250))
      await page.evaluate(() => document.querySelector('#same').remove())
      return { waited: await waiting }`,
    expected: { waited: failed('error') },
  },
  {
    name: 'an abort stops the wait and its polling in an out-of-process frame',
    source: `
      const frame = ${byName('cross')}
      const controller = new AbortController()
      const waiting = settle(() => frame.waitForFunction(() => { window.__ticks = (window.__ticks || 0) + 1; return false }, undefined, { polling: 20, signal: controller.signal, timeout: 0 }))
      await new Promise((resolve) => setTimeout(resolve, 200))
      controller.abort('stop')
      const waited = await waiting
      const before = await frame.evaluate(() => window.__ticks)
      await new Promise((resolve) => setTimeout(resolve, 150))
      return { waited, stopped: before === (await frame.evaluate(() => window.__ticks)) }`,
    expected: {
      waited: { error: 'error', name: 'AbortError' },
      stopped: true,
    },
  },
]

test('frame.waitForFunction keeps waiting across its frame process changes', {
  skip,
  timeout: 600_000,
}, async (t) => {
  await compareFrameCases(t, cases)
})
