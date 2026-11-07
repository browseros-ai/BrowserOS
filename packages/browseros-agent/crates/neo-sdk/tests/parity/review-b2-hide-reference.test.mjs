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
    const hiding = page.hideHighlight()
    await Promise.all([blocked, childHidden])
    await page.frame('B').locator('#new').highlight()
    releaseMain()
    await hiding
    mainGlue.evaluate = mainEvaluate
    childGlue.evaluate = childEvaluate
    const cdp = await context.newCDPSession(page.frame('B'))
    const walk = (n) => [
      n,
      ...(n.children ?? []).flatMap(walk),
      ...(n.shadowRoots ?? []).flatMap(walk),
    ]
    const count = walk(
      (await cdp.send('DOM.getDocument', { depth: -1, pierce: true })).root,
    ).filter((n) => n.nodeName === 'X-PW-HIGHLIGHT').length
    t.diagnostic(JSON.stringify({ afterOldHide: count }))
    assert.equal(count, 1)
  } finally {
    releaseMain()
    await runCleanups(cleanups)
  }
})
