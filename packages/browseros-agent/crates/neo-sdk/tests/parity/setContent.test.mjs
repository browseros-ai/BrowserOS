// Page.setContent and Frame.setContent through Neo and Playwright: the written markup replaces
// the document (its scripts run in the page, its <title> becomes the title), each `waitUntil`
// resolves once the new content fires it, a child frame's document can be written alone, and
// options are settled as Playwright's client settles them (a `null` options object throws its
// TypeError reading `waitUntil`; a `null` or unknown `waitUntil` and a `null` html are Errors).
// With a slow image, `domcontentloaded` does not wait for the image and `load` times out at the
// given timeout, in both engines.
//
// From Playwright's specs at the pin (tests/page/page-set-content.spec.ts at 26137face): 'PA-020
// should work', 'PA-021 should work with domcontentloaded', 'PA-022 should work with commit',
// 'PA-026 should respect timeout', 'PA-028 should await resources to load' and 'PA-029 should
// work fast enough' (several writes in a row).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const ISOLATE = `<script>new MutationObserver(() => document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement, { childList: true, subtree: true })</script>`

test('setContent writes the document and waits for its lifecycle events', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: `<p id="p">original</p>${ISOLATE}`,
    source: `
      const voids = []
      voids.push(await page.setContent('<title>first</title><p id="p">one</p><script>window.__ran = (window.__ran || 0) + 1</script>'))
      const first = { title: await page.title(), text: await page.locator('#p').textContent() }
      voids.push(await page.mainFrame().setContent('<title>second</title><p id="p">two</p>', { waitUntil: 'domcontentloaded' }))
      const second = { title: await page.title(), text: await page.locator('#p').textContent() }
      voids.push(await page.setContent('<p id="p">three</p>', { waitUntil: 'commit' }))
      const third = await page.locator('#p').textContent()
      for (let i = 0; i < 5; i++) voids.push(await page.setContent('<p id="p">fast ' + i + '</p>', { waitUntil: 'load' }))
      const settled = []
      for (const [html, options] of [['<p>x</p>', null], ['<p>x</p>', { waitUntil: null }], ['<p>x</p>', { waitUntil: 'bogus' }], [null, undefined]]) {
        try { await page.setContent(html, options); settled.push('resolved') } catch (error) { settled.push(errorCategory(error)) }
      }
      await page.setContent('<title>last</title><p id="p">last</p><script>window.__ran = (window.__ran || 0) + 10</script>')
      return { voids: voids.map((value) => typeof value), first, second, third, settled }
    `,
    probe:
      '({ ran: window.__ran, title: document.title, text: document.body.textContent })',
  })
  assert.deepEqual(outcome.value, {
    voids: Array(8).fill('undefined'),
    first: { title: 'first', text: 'one' },
    second: { title: 'second', text: 'two' },
    third: 'three',
    settled: ['TypeError', 'Error', 'Error', 'Error'],
  })
})

test('setContent on a child frame writes only that frame', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: `<p id="outer">outer</p><iframe name="child" srcdoc="<p id='p'>original</p>"></iframe>${ISOLATE}`,
    source: `
      const child = page.frame('child')
      await child.locator('#p').waitFor()
      const written = await child.setContent('<p id="p">replaced</p>')
      return {
        written: typeof written,
        child: await child.locator('#p').textContent(),
        outer: await page.locator('#outer').textContent(),
      }
    `,
    probe:
      "document.querySelector('iframe').contentDocument.querySelector('#p').textContent",
  })
  assert.deepEqual(outcome.value, {
    written: 'undefined',
    child: 'replaced',
    outer: 'outer',
  })
})

test('setContent waits for load unless told otherwise, under its timeout', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: `<p>start</p>${ISOLATE}`,
    // The fixture's `/slow` answers after this long, holding the image's load.
    slowMs: 4000,
    source: `
      const settled = {}
      for (const [name, html, options] of [
        ['domcontentloaded', '<p>quick</p><img src="/slow">', { waitUntil: 'domcontentloaded' }],
        ['load', '<p>slow</p><img src="/slow?again">', { timeout: 500 }],
      ]) {
        try { settled[name] = typeof await page.setContent(html, options) } catch (error) { settled[name] = errorCategory(error) }
      }
      return settled
    `,
    probe: "document.querySelector('p')?.textContent",
  })
  assert.deepEqual(outcome.value, {
    domcontentloaded: 'undefined',
    load: 'TimeoutError',
  })
})
