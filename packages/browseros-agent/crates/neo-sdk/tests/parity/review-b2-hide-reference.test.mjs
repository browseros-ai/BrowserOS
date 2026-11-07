// Pinned Playwright's reference for review-b2-recheck.test.mjs 'review: an older hide-all keeps
// a newer highlight in another document', which holds Neo to this count: a main-frame hide still
// pending does not erase a highlight that frame B drew after B's own hide.
//
// Playwright's setHighlights draws on the frame's next animation frame (injected highlight.ts
// _ensureElementHighlightRaf), so locator.highlight() resolves before the element exists. The
// counts below come after B's animation frames, never after a timer.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  browserBinary,
  chromium,
  launchBrowser,
  runCleanups,
  serveFixture,
  skip,
} from './harness.mjs'

test('pinned Playwright keeps a new B highlight while main-frame hide is pending', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const cleanups = []
  let releaseMain = () => {}
  try {
    const origin = await serveFixture(cleanups, {
      pages: {
        '/main':
          '<button id="main">Main</button><iframe name="B" id="b"></iframe><script>b.src="http://localhost:"+location.port+"/b"</script>',
        '/b': '<button id="old">Old</button><button id="new">New</button>',
      },
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const pw = await chromium.connectOverCDP(`http://127.0.0.1:${browser.port}`)
    cleanups.push(() => pw.close())
    const [context] = pw.contexts(),
      page = await context.newPage()
    await page.goto(origin + '/main')
    await page.locator('#main').highlight()
    await page.frameLocator('#b').locator('#old').highlight()
    const main = page._connection.toImpl(page.mainFrame())
    const child = page._connection.toImpl(page.frame('B'))
    const mainGlue = await (await main.context('utility')).injectedScript()
    const childGlue = await (await child.context('utility')).injectedScript()
    const mainEvaluate = mainGlue.evaluate.bind(mainGlue),
      childEvaluate = childGlue.evaluate.bind(childGlue)
    let onBlocked, onChildHidden
    const blocked = new Promise((resolve) => (onBlocked = resolve))
    const childHidden = new Promise((resolve) => (onChildHidden = resolve))
    const gate = new Promise((resolve) => (releaseMain = resolve))
    let once = false
    mainGlue.evaluate = async (fn, ...args) => {
      if (!once && String(fn).includes('hideHighlight')) {
        once = true
        onBlocked()
        await gate
      }
      return mainEvaluate(fn, ...args)
    }
    childGlue.evaluate = async (fn, ...args) => {
      const result = await childEvaluate(fn, ...args)
      if (String(fn).includes('hideHighlight')) onChildHidden()
      return result
    }
    const b = page.frame('B')
    const cdp = await context.newCDPSession(b)
    const walk = (n) => [
      n,
      ...(n.children ?? []).flatMap(walk),
      ...(n.shadowRoots ?? []).flatMap(walk),
    ]
    const count = async () =>
      walk(
        (await cdp.send('DOM.getDocument', { depth: -1, pierce: true })).root,
      ).filter((n) => n.nodeName === 'X-PW-HIGHLIGHT').length
    const nextFrame = () =>
      b.evaluate(
        () => new Promise((resolve) => requestAnimationFrame(() => resolve())),
      )
    const hiding = page.hideHighlight()
    await Promise.all([blocked, childHidden])
    await b.locator('#new').highlight()
    // The new highlight is drawn while the main-frame hide is still pending.
    let drawn = 0
    for (let frames = 0; !drawn && frames < 60; frames++) {
      await nextFrame()
      drawn = await count()
    }
    releaseMain()
    await hiding
    mainGlue.evaluate = mainEvaluate
    childGlue.evaluate = childEvaluate
    // Whatever the old hide did to B has landed by B's next frame.
    await nextFrame()
    const afterOldHide = await count()
    t.diagnostic(JSON.stringify({ drawn, afterOldHide }))
    assert.equal(
      drawn,
      1,
      'the new B highlight is drawn before the old hide ends',
    )
    assert.equal(
      afterOldHide,
      1,
      'the old main-frame hide keeps the new B highlight',
    )
  } finally {
    releaseMain()
    await runCleanups(cleanups)
  }
})
