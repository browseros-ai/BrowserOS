import assert from 'node:assert/strict'
import test from 'node:test'
import { screenshotReceiver } from './visual-screenshot.mjs'

// The pinned server algorithm is exercised by the unchanged upstream screenshot specs.
// This seam verifies that its only capture callbacks call the actual Neo receiver.
function serverWith(expectScreenshot) {
  return {
    nullProgress: { metadata: {} },
    Page: { prototype: { expectScreenshot } },
  }
}

test('page and locator visual callbacks capture Neo pixels and preserve explicit masks', async () => {
  const pixels = Buffer.from('neo pixels')
  const calls = []
  const page = {
    isClosed: () => false,
    screenshot: async (options) => {
      calls.push(['page', options])
      return pixels
    },
  }
  const locator = {
    page: () => page,
    toString: () => 'locator(#node)',
    screenshot: async (options) => {
      calls.push(['locator', options])
      return pixels
    },
  }
  const server = serverWith(async function (progress, options) {
    const actual = options.locator
      ? await options.locator.frame.rafrafTimeoutScreenshotElementWithProgress(
          progress,
          '',
          0,
        )
      : await this.screenshotter.screenshotPage(progress, options)
    return { actual }
  })
  const options = {
    mask: [locator],
    maskColor: 'red',
    scale: 'css',
    expected: Buffer.from('baseline'),
    maxDiffPixels: 4,
    timeout: 1000,
  }
  assert.equal(
    (await screenshotReceiver(page, server)._expectScreenshot(options)).actual,
    pixels,
  )
  const visualLocator = screenshotReceiver(locator, server)
  assert.equal(visualLocator._apiName, 'Locator')
  assert.equal(
    (
      await visualLocator
        .page()
        ._expectScreenshot({ ...options, locator: visualLocator })
    ).actual,
    pixels,
  )
  assert.deepEqual(
    calls.map(([subject]) => subject),
    ['page', 'locator'],
  )
  for (const [, capture] of calls) {
    assert.equal(capture.mask, options.mask)
    assert.equal(capture.maskColor, 'red')
    assert.equal(capture.scale, 'css')
    assert.equal('expected' in capture, false)
    assert.equal('maxDiffPixels' in capture, false)
    assert.ok(capture.timeout > 0 && capture.timeout <= 1000)
  }
})

test('assertion deadlines and caller cancellation abort the pending Neo capture', async () => {
  let signal
  let started
  const page = {
    isClosed: () => false,
    screenshot: async (options) => {
      signal = options.signal
      started?.()
      return new Promise(() => {})
    },
  }
  const server = serverWith(async function (progress, options) {
    return this.screenshotter.screenshotPage(progress, options)
  })
  const receiver = screenshotReceiver(page, server)
  await assert.rejects(
    receiver._expectScreenshot({ timeout: 10 }),
    /Timeout 10ms/,
  )
  assert.equal(signal.aborted, true)
  const caller = new AbortController()
  const entered = new Promise((resolve) => {
    started = resolve
  })
  const pending = receiver._expectScreenshot({
    timeout: 1000,
    signal: caller.signal,
  })
  await entered
  caller.abort('stopped')
  await assert.rejects(pending, /Assertion aborted: stopped/)
  assert.equal(signal.aborted, true)
})
