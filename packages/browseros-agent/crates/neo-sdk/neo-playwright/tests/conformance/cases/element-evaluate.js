// The element scripts as Playwright runs them: locator.evaluate waits for its element (one
// added later) and is strict; $eval takes the first match now and quotes the selector as the
// script wrote it when there is none; $$eval and evaluateAll take every match now and, with no
// match, run fn([]) in the frame the query starts from, for a reached frame and for a missing
// hop alike; elementHandle.$eval and $$eval query from the handle; and a removed element's
// handle still runs, where a query rooted at one runs in upstream only. Upstream serializes
// locator.evaluate's argument once its element is found, so a change the script makes after the
// call reaches the page; and it takes an abort signal for that wait. Neo's runtime has no
// AbortController, so there the signal is a stand-in object, which Neo refuses as it would a
// real one.
await page.goto(fixtures.url('frames.html'))
await harness.poll(page, "!!document.querySelector('iframe[name=same]').contentDocument?.querySelector('#name')")
const keep = (work) => harness.settle(work)
const where = (els) => [els.length, document.title]
await harness.pageEval(
  page,
  "setTimeout(() => document.body.insertAdjacentHTML('beforeend', '<p id=late>late</p>'), 300)",
)
const top = await page.$('body')
const button = await page.$('button')
await harness.pageEval(page, "document.querySelector('button').remove()")
await harness.pageEval(page, "document.body.insertAdjacentHTML('beforeend', '<div id=box><span>inside</span></div>')")
const box = await page.$('#box')
await harness.pageEval(page, "document.getElementById('box').remove()")
return {
  waited: await keep(page.locator('#late').evaluate((el) => el.textContent)),
  strict: await keep(page.locator('iframe').evaluate((el) => el.name)),
  first: await keep(page.$eval('iframe', (el) => el.name)),
  noMatch: await keep(page.$eval('#nope', (el) => el)),
  noMatchInFrame: await keep(
    page.$eval('iframe[name=same] >> internal:control=enter-frame >> #nope', (el) => el),
  ),
  all: await keep(page.$$eval('iframe', (els) => els.map((el) => el.name))),
  reachedNoMatch: await keep(page.frameLocator('iframe[name=same]').locator('.none').evaluateAll(where)),
  missingHop: await keep(page.frameLocator('#missing').locator('div').evaluateAll(where)),
  inFrame: await keep(page.frameLocator('iframe[name=same]').locator('#name').evaluateAll((els) => [els.length, document.title, els[0].textContent])),
  fromChild: await keep(
    page.frame({ name: 'same' }).locator('.none').evaluateAll(where),
  ),
  handleEval: await keep(top.$eval('h1', (el) => el.textContent)),
  handleNoMatch: await keep(top.$eval('.none', (el) => el)),
  handleAll: await keep(top.$$eval('iframe', (els) => els.length)),
  removed: await keep(button.evaluate((el) => [el.isConnected, el.textContent])),
  detachedRoot: await keep(box.$eval('span', (el) => el.textContent)),
  detachedRootAll: await keep(box.$$eval('span', (els) => els.length)),
  argAtCall: await keep(
    (async () => {
      const arg = { n: 1 }
      const pending = page.locator('h1').evaluate((el, a) => a.n, arg)
      arg.n = 2
      return await pending
    })(),
  ),
  signal: await keep(
    page.locator('h1').evaluate((el) => el.tagName, undefined, {
      signal: typeof AbortController === 'function' ? new AbortController().signal : {},
    }),
  ),
}
