// page.on('domcontentloaded'), page.on('load') and context.on('pageload') through the pinned
// Playwright and through Neo, on fresh tests/pages/load-events.html tabs (events.mjs): a goto
// fires the page's 'domcontentloaded', then its 'load', then the context's 'pageload', each with
// the page, once per document and all before goto resolves (client/frame.ts's 'loadstate'
// handler); a link's navigation fires the same three, which waitForEvent('load') and the
// context's waitForEvent('pageload') resolve with, while a same-document one (#section) fires
// none; and a child frame's navigation fires none either: only the main frame's do. Each event
// records page.url() as its listener sees it, by path (`path`).
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

/** Records the three events in `seen`; `stop()` takes the listeners off. */
const LISTEN = `const seen = []
    const onDomContentLoaded = (loaded) => seen.push(['domcontentloaded', loaded === page, path(page.url())])
    const onLoad = (loaded) => seen.push(['load', loaded === page, path(page.url())])
    const onPageLoad = (loaded) => {
      if (loaded === page) seen.push(['pageload', path(loaded.url())])
    }
    page.on('domcontentloaded', onDomContentLoaded)
    page.on('load', onLoad)
    page.context().on('pageload', onPageLoad)
    const stop = () => {
      page.off('domcontentloaded', onDomContentLoaded)
      page.off('load', onLoad)
      page.context().off('pageload', onPageLoad)
    }
    `
/** What LISTEN records for one document at `at`. */
const loaded = (at) => [
  ['domcontentloaded', true, at],
  ['load', true, at],
  ['pageload', at],
]

const cases = [
  {
    // tests/page/page-event-load.spec.ts "NA-087 should fire once";
    // tests/library/browsercontext-events.spec.ts "BC-045 pageload event should work @smoke".
    name: "a goto fires 'domcontentloaded', 'load' and 'pageload' once each before it resolves",
    path: '/load-events.html',
    source: `${LISTEN}await page.goto(page.url() + '?second')
    const atGoto = seen.splice(0)
    await page.goto(page.url().replace('?second', '?third'))
    const atNextGoto = seen.splice(0)
    stop()
    return { atGoto, atNextGoto }`,
    expected: {
      atGoto: loaded('/load-events.html?second'),
      atNextGoto: loaded('/load-events.html?third'),
    },
    probe: 'location.search',
    effect: '?third',
  },
  {
    // page-event-load.spec.ts "NA-087 should fire once"; browsercontext-events.spec.ts "BC-045
    // pageload event should work @smoke" (`context.waitForEvent('pageload')` is the page).
    name: "a link's navigation fires the three, and the waits get the page; a same-document one none",
    path: '/load-events.html',
    source: `${LISTEN}const [fromLoad] = await Promise.all([page.waitForEvent('load'), page.click('#next')])
    const afterLink = seen.splice(0)
    await page.click('#section')
    await page.waitForURL(/#section$/)
    const afterSection = seen.splice(0)
    const [fromPageLoad] = await Promise.all([
      page.context().waitForEvent('pageload'),
      page.goto(page.url().replace('?next#section', '?third')),
    ])
    const afterGoto = seen.splice(0)
    stop()
    return { afterLink, afterSection, afterGoto, waited: [fromLoad === page, fromPageLoad === page] }`,
    expected: {
      afterLink: loaded('/load-events.html?next'),
      afterSection: [],
      afterGoto: loaded('/load-events.html?third'),
      waited: [true, true],
    },
    probe: 'location.search',
    effect: '?third',
  },
  {
    // page-event-load.spec.ts "NA-088 should fire once with iframe navigation".
    name: "a child frame's navigation fires none of them; the next goto fires each once",
    path: '/load-events.html',
    source: `${LISTEN}const child = page.frame({ name: 'child' })
    const before = path(child.url())
    const [navigated] = await Promise.all([
      page.waitForEvent('framenavigated', (frame) => path(frame.url()) === '/child-2'),
      page.click('#navigate-frame'),
    ])
    await child.waitForLoadState()
    const childLoaded = [navigated === child, path(child.url()), await child.title()]
    const afterChild = seen.splice(0)
    await page.goto(page.url() + '?second')
    const afterGoto = seen.splice(0)
    stop()
    return { before, childLoaded, afterChild, afterGoto }`,
    expected: {
      before: '/child-1',
      childLoaded: [true, '/child-2', '/child-2'],
      afterChild: [],
      afterGoto: loaded('/load-events.html?second'),
    },
    probe: 'location.search',
    effect: '?second',
  },
]

test('load events fire in the same order with the same Page through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: { '/load-events.html': fixture('load-events.html') },
    cases,
  })
})
