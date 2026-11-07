// Page.waitForSelector and Frame.waitForSelector through Neo and Playwright: the first match
// meeting `state` comes back as an ElementHandle the page can read (attached, visible, in a
// shadow root, through `>>`), `hidden` and `detached` waits answer null, Page waits in the main
// frame and a Frame in its own; the legacy `waitFor` and `visibility` options, invalid states,
// strictness, a timeout, `null` options and a removed frame fail alike. Values by `describe`
// (values.mjs), errors by category and the words Playwright's message holds.
//
// From Playwright's specs at the pin (tests/page/page-wait-for-selector-1.spec.ts and -2.spec.ts
// at 26137face): WA-027, WA-028, WA-029, WA-035, WA-039, WA-040, WA-041, WA-042, WA-043, WA-045,
// WA-049, WA-051, WA-052, WA-053, WA-056, WA-058, WA-059, WA-060, WA-061, WA-062, WA-063, WA-064.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { SETTLED, VALUES } from './values.mjs'

/** Keeps BrowserOS's takeover UI out of the documents, so selectors match the fixture alone. */
const ISOLATE = `<script>new MutationObserver(() => document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement, { childList: true, subtree: true })</script>`

const HTML = `<!doctype html><html><body>
<article id="first">first</article>
<iframe name="frame1" srcdoc="<body></body>"></iframe>
<iframe name="frame2" srcdoc="<body></body>"></iframe>
${ISOLATE}
</body></html>`

/** Helpers every case shares: add an element, read a handle back from the page. */
const HELPERS = `${VALUES}${SETTLED}
const add = (target, tag, className = '') => target.evaluate(([tag, className]) => {
  const element = document.createElement(tag)
  element.className = className
  document.body.appendChild(element)
}, [tag, className])
const read = (handle) => handle && handle.evaluate((e) => e.tagName + ':' + (e.className || e.id) + '@' + (window.name || 'main'))
await page.frame('frame2').waitForSelector('body', { state: 'attached' })
`

test('waitForSelector answers the element that meets the state, or null when hidden or detached', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: HTML,
    source: `${HELPERS}
      const frame = page.mainFrame()
      return {
        // WA-029: an existing node at once, then one added later.
        immediate: await outcome(async () => read(await frame.waitForSelector('article'))),
        attached: await outcome(async () => {
          const watchdog = frame.waitForSelector('section.late', { state: 'attached' })
          await add(page, 'br')
          await add(page, 'section', 'late')
          return (await (await watchdog).getProperty('tagName')).jsonValue()
        }),
        // WA-039: a node added in an open shadow root.
        shadow: await outcome(async () => {
          const watchdog = page.waitForSelector('span.shadowed')
          await page.evaluate(() => {
            const host = document.createElement('aside')
            host.attachShadow({ mode: 'open' })
            document.body.appendChild(host)
          })
          await new Promise((resolve) => setTimeout(resolve, 100))
          await page.evaluate(() => {
            const span = document.createElement('span')
            span.className = 'shadowed'
            span.textContent = 'Hello from shadow'
            document.querySelector('aside').shadowRoot.appendChild(span)
          })
          return (await watchdog).evaluate((e) => e.textContent)
        }),
        // WA-040: a node that innerHTML brings in.
        innerHTML: await outcome(async () => {
          const watchdog = page.waitForSelector('h3 b', { state: 'attached' })
          await add(page, 'nav', 'holder')
          await page.evaluate(() => { document.querySelector('nav.holder').innerHTML = '<h3><b>bold</b></h3>' })
          return read(await watchdog)
        }),
        // WA-045: a hidden node becomes visible.
        visible: await outcome(async () => {
          await page.evaluate(() => { document.body.insertAdjacentHTML('beforeend', '<main class="shy" style="display: none">shy</main>') })
          const watchdog = page.waitForSelector('main.shy')
          await page.evaluate(() => { document.querySelector('main.shy').style.removeProperty('display') })
          return read(await watchdog)
        }),
        // WA-049 and WA-051: a hidden wait answers null once hidden, and once removed.
        hidden: await outcome(async () => {
          const watchdog = page.waitForSelector('main.shy', { state: 'hidden' })
          await page.evaluate(() => { document.querySelector('main.shy').style.setProperty('visibility', 'hidden') })
          return watchdog
        }),
        removed: await outcome(async () => {
          const watchdog = page.waitForSelector('main.shy', { state: 'hidden' })
          await page.evaluate(() => document.querySelector('main.shy').remove())
          return watchdog
        }),
        // WA-052, WA-063, WA-064: nothing to hide, already detached, then detached.
        hideMissing: await outcome(() => page.waitForSelector('non-existing', { state: 'hidden' })),
        detachedAlready: await outcome(() => page.waitForSelector('non-existing', { state: 'detached' })),
        detached: await outcome(async () => {
          const watchdog = page.waitForSelector('section.late', { state: 'detached' })
          await page.evaluate(() => document.querySelector('section.late').remove())
          return watchdog
        }),
        // WA-056 and WA-062: the handle itself, and a chained selector.
        handle: await outcome(async () => settled(await page.waitForSelector('article#first'))),
        chained: await outcome(async () => read(await page.waitForSelector('nav.holder >> h3 >> b'))),
        notStrict: await outcome(async () => read(await page.waitForSelector('iframe', { state: 'attached' }))),
      }
    `,
    probe: 'document.body.children.length',
  })
  assert.deepEqual(outcome.value.immediate, { value: 'ARTICLE:first@main' })
  assert.deepEqual(outcome.value.attached, { value: 'SECTION' })
  assert.deepEqual(outcome.value.shadow, { value: 'Hello from shadow' })
  assert.deepEqual(outcome.value.hidden, { value: null })
  assert.deepEqual(outcome.value.removed, { value: null })
  assert.deepEqual(outcome.value.hideMissing, { value: null })
  assert.deepEqual(outcome.value.detached, { value: null })
})

test('waitForSelector waits in its own frame and fails alike', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: HTML,
    source: `${HELPERS}
      const frame1 = page.frame('frame1')
      const frame2 = page.frame('frame2')
      return {
        // WA-041: Page waits in the main frame, not in a child that gets the node first.
        page: await outcome(async () => {
          const watchdog = page.waitForSelector('section.both', { state: 'attached' })
          await add(frame1, 'section', 'both')
          await add(page, 'section', 'both')
          return read(await watchdog)
        }),
        // WA-042: a Frame waits in its own document.
        frame: await outcome(async () => {
          const watchdog = frame2.waitForSelector('section.child', { state: 'attached' })
          await add(frame1, 'section', 'child')
          await add(frame2, 'section', 'child')
          return read(await watchdog)
        }),
        // WA-027, WA-028, WA-059: the options of older releases.
        waitForAttached: await outcome(() => page.waitForSelector('*', { waitFor: 'attached' }), ['options.waitFor is not supported']),
        waitForVisible: await outcome(async () => read(await page.waitForSelector('article', { waitFor: 'visible' }))),
        visibility: await outcome(() => page.waitForSelector('article', { visibility: 'hidden' }), ['options.visibility is not supported']),
        // WA-058, WA-060, WA-061: states the protocol refuses.
        unknownState: await outcome(() => page.waitForSelector('article', { state: 'foo' })),
        trueState: await outcome(() => page.waitForSelector('article', { state: true })),
        falseState: await outcome(() => page.waitForSelector('article', { state: false })),
        // WA-053 and strictness.
        timeout: await outcome(() => page.waitForSelector('div.missing', { timeout: 300 }), ['300ms']),
        strict: await outcome(() => page.waitForSelector('iframe', { strict: true, state: 'attached', timeout: 1000 })),
        nullOptions: await outcome(() => page.waitForSelector('article', null)),
        frameNullOptions: await outcome(() => frame1.waitForSelector('body', null)),
        // WA-043: the frame is removed while its wait runs. The outcome is settled as the wait
        // starts, so its rejection is handled before the removal lands.
        frameDetached: await (async () => {
          const watchdog = frame1.waitForSelector('.box').then(
            (value) => ({ value: describe(value) }),
            (error) => ({ error: errorCategory(error) }),
          )
          await page.evaluate(() => document.querySelector('iframe[name=frame1]').remove())
          return watchdog
        })(),
      }
    `,
    probe: 'document.querySelectorAll("iframe").length',
  })
  assert.deepEqual(outcome.value.page, { value: 'SECTION:both@main' })
  assert.deepEqual(outcome.value.frame, { value: 'SECTION:child@frame2' })
  assert.equal(outcome.value.waitForAttached.error, 'Error')
  assert.equal(outcome.value.visibility.error, 'Error')
  assert.equal(outcome.value.timeout.error, 'TimeoutError')
  assert.equal(outcome.value.strict.error, 'Error:strict')
  assert.equal(outcome.value.frameDetached.error, 'Error')
})
