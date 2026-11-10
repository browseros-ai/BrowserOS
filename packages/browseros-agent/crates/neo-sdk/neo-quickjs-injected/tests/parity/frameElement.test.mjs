// Frame.frameElement through Neo and Playwright: each child frame's owner comes back as an
// ElementHandle of its parent document, the very element `page.$` finds, for an `<iframe>`, an
// `<iframe>` in a closed or a declarative shadow root, and a `<frame>` of a frameset; the main
// frame and a removed frame have been detached. Values by `describe` (values.mjs), errors by
// category and the words Playwright's message holds.
//
// From Playwright's specs at the pin (tests/page/frame-frame-element.spec.ts at 26137face):
// 'FR-014 should work @smoke', 'FR-016 should work with frameset', 'FR-017 should throw when
// detached', 'FR-018 should work inside closed shadow root' and 'FR-019 should work inside
// declarative shadow root'. FR-015 reads `ElementHandle.contentFrame`, which Neo does not serve.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { SETTLED, VALUES } from './values.mjs'

/** Keeps BrowserOS's takeover UI out of the documents, so selectors match the fixture alone. */
const ISOLATE = `<script>new MutationObserver(() => document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement, { childList: true, subtree: true })</script>`

const HTML = `<!doctype html><html><body>
<iframe id="frame1" name="frame1" srcdoc="<p>one</p>"></iframe>
<iframe id="frame2" name="frame2" srcdoc="<p>two</p>"></iframe>
<iframe id="frame3" name="frame3" srcdoc="<p>three</p>"></iframe>
<div id="closed"></div>
<div id="declarative"><template shadowrootmode="open"><iframe name="declared" srcdoc="<h1>Hi!</h1>"></iframe><slot></slot></template><span>footer</span></div>
<script>
  const owner = document.createElement('iframe')
  owner.setAttribute('name', 'myframe')
  owner.setAttribute('srcdoc', 'find me')
  document.getElementById('closed').attachShadow({ mode: 'closed' }).appendChild(owner)
</script>
${ISOLATE}
</body></html>`

test("frameElement answers each child frame's owner element", {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: HTML,
    source: `${VALUES}${SETTLED}
      for (const name of ['frame1', 'frame2', 'frame3', 'myframe', 'declared'])
        await page.frame(name).waitForSelector('body', { state: 'attached' })
      const name = async (frame) => (await page.frame(frame).frameElement()).evaluate((e) => e.getAttribute('name'))
      const one = await page.$('#frame1')
      const three = await page.$('#frame3')
      return {
        // FR-014: the owner is the element page.$ finds, and no other.
        same1: await outcome(async () => one.evaluate((a, b) => a === b, await page.frame('frame1').frameElement())),
        same3: await outcome(async () => three.evaluate((a, b) => a === b, await page.frame('frame3').frameElement())),
        different: await outcome(() => one.evaluate((a, b) => a === b, three)),
        preview: await outcome(async () => settled(await page.frame('frame2').frameElement())),
        tag: await outcome(async () => (await page.frame('frame2').frameElement()).evaluate((e) => e.tagName + ' in ' + (window.name || 'main'))),
        // FR-018 and FR-019: owners inside shadow roots.
        closed: await outcome(() => name('myframe')),
        declarative: await outcome(() => name('declared')),
        // The main frame has no owner.
        main: await outcome(() => page.mainFrame().frameElement(), ['Frame has been detached.']),
        // FR-017: a removed frame has been detached.
        detached: await outcome(async () => {
          const frame = page.frame('frame2')
          await page.$eval('#frame2', (e) => e.remove())
          return frame.frameElement()
        }, ['Frame has been detached.']),
      }
    `,
    probe: 'document.querySelectorAll("iframe").length',
  })
  assert.deepEqual(outcome.value.same1, { value: true })
  assert.deepEqual(outcome.value.same3, { value: true })
  assert.deepEqual(outcome.value.different, { value: false })
  assert.deepEqual(outcome.value.tag, { value: 'IFRAME in main' })
  assert.deepEqual(outcome.value.closed, { value: 'myframe' })
  assert.deepEqual(outcome.value.declarative, { value: 'declared' })
  assert.equal(outcome.value.main.error, 'Error')
  assert.equal(outcome.value.detached.error, 'Error')
})

test("frameElement answers a frameset's frame (FR-016)", {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: `<!doctype html><html><frameset cols="50%,50%"><frame name="left" src="about:blank"><frame name="right" src="about:blank"></frameset></html>`,
    source: `${VALUES}${SETTLED}
      await page.frame('right').waitForSelector('body', { state: 'attached' })
      const first = await page.$('frame')
      return {
        same: await outcome(async () => first.evaluate((a, b) => a === b, await page.frame('left').frameElement())),
        other: await outcome(async () => first.evaluate((a, b) => a === b, await page.frame('right').frameElement())),
        tag: await outcome(async () => (await page.frame('right').frameElement()).evaluate((e) => e.tagName + ':' + e.name)),
      }
    `,
    probe: 'document.querySelectorAll("frame").length',
  })
  assert.deepEqual(outcome.value, {
    same: { value: true },
    other: { value: false },
    tag: { value: 'FRAME:right' },
  })
})
