// Two exposed-binding cases from med-pw-int's batch-3 delta review (review-delta-88bd0df65),
// through the Playwright reference and through Neo (each in a browser of its own: Playwright's
// context-wide binding would answer Neo's page too).
//
// F1: a call read late, after its document navigated away, answers only that document. The old
// document calls `delayed('old')` and navigates at once (to `localhost` for another process);
// the new one calls `delayed('new')`. Both callbacks are held, the old answer goes first: the
// new call still waits, and ends with its own answer. `NEO_TEST_BINDING_POLL_PAUSE_MS` (a
// debug-only hook in bindings.rs) holds Neo's binding reader, as a preemption would, so it
// reads the old call after the navigation's events; before the fix, a process change let the
// old answer reach the new document.
//
// F3: a binding answers while the run's 64-call cap is full. 63 `waitForTimeout` calls and the
// evaluation that calls the binding fill the cap; the callback's answer still reaches the page
// at once, for a page's binding and for the context's.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const LATE_HTML = `<!doctype html><title>late origin</title><p>top</p>
<script>
if (location.search.includes('new')) {
  window.result = 'pending'
  window.answer = window.delayed('new').then((value) => (window.result = value))
}
</script>`

const pause = (ms) => `new Promise((resolve) => setTimeout(resolve, ${ms}))`

const lateSource = (cross) => `
  let releaseOld
  let releaseNew
  let oldReturned = false
  await page.exposeFunction('delayed', async (which) => {
    const value = await new Promise((resolve) => {
      if (which === 'old') releaseOld = resolve
      else releaseNew = resolve
    })
    if (which === 'old') oldReturned = true
    return value
  })
  const url = (page.url().split('?')[0] + '?new=1')${cross ? ".replace('127.0.0.1', 'localhost')" : ''}
  await page
    .evaluate((url) => {
      window.delayed('old').catch(() => {})
      location.href = url
    }, url)
    .catch(() => {})
  await page.waitForURL(/new=1/, { timeout: 6000 })
  for (let i = 0; i < 400 && (!releaseOld || !releaseNew); i++) await ${pause(10)}
  if (!releaseOld || !releaseNew) throw new Error('both callbacks must arrive')
  releaseOld('old-answer')
  for (let i = 0; i < 400 && !oldReturned; i++) await ${pause(10)}
  // Time for an answer that went to the wrong document to land there.
  await ${pause(300)}
  const beforeNew = await page.evaluate(() => window.result)
  releaseNew('new-answer')
  const result = await page.evaluate(() => window.answer)
  return { beforeNew, result }
`

for (const cross of [true, false]) {
  test(`a binding call read after its document ${cross ? 'changed process' : 'navigated'} answers only that document`, {
    skip,
    timeout: 180_000,
  }, async (t) => {
    process.env.NEO_TEST_BINDING_POLL_PAUSE_MS = '150'
    try {
      await compareMethod(t, {
        html: LATE_HTML,
        isolate: true,
        source: lateSource(cross),
        probe: 'document.title',
        values({ playwright, neo }) {
          t.diagnostic(JSON.stringify({ playwright, neo }))
          assert.deepEqual(playwright, {
            beforeNew: 'pending',
            result: 'new-answer',
          })
          assert.deepEqual(neo, playwright)
        },
      })
    } finally {
      delete process.env.NEO_TEST_BINDING_POLL_PAUSE_MS
    }
  })
}

for (const owner of ['page', 'page.context()']) {
  test(`a binding of ${owner} answers while the call cap is full`, {
    skip,
    timeout: 180_000,
  }, async (t) => {
    await compareMethod(t, {
      html: '<!doctype html><title>binding cap</title><p>cap</p>',
      isolate: true,
      source: `
        let callbacks = 0
        await ${owner}.exposeFunction('capBinding', () => {
          callbacks++
          return 37
        })
        const waiting = Array.from({ length: 63 }, () => page.waitForTimeout(1400))
        const result = await Promise.race([
          page.evaluate(() => window.capBinding()).then(
            (value) => ({ value }),
            (error) => ({ error: error.name }),
          ),
          ${pause(2200)}.then(() => ({ unsettled: true })),
        ])
        await Promise.all(waiting)
        return { result, callbacks }
      `,
      probe: 'document.title',
      values({ playwright, neo }) {
        t.diagnostic(JSON.stringify({ playwright, neo }))
        assert.deepEqual(playwright, { result: { value: 37 }, callbacks: 1 })
        assert.deepEqual(neo, playwright)
      },
    })
  })
}
