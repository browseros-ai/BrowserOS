// page.opener() through the pinned Playwright and through Neo, on fresh tests/pages/opener.html
// tabs (events.mjs): a popup's opener is the page that opened it, and a popup's own popup names
// that popup, not the page, whose 'popup' it never reaches; the run's page and a
// context.newPage() page have none, while a popup that the new page opens names it; and once a
// popup's opener has closed, opener() is null, though a closed popup still names its open
// opener (client/page.ts `opener` looks only at the opener). Each case closes what it opened
// before it returns: a tab outlives Playwright's disconnect and would still be open through
// Neo's turn. URLs compare by path (`path`).
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    // tests/library/browsercontext-page-event.spec.ts "BC-069 should have an opener";
    // tests/page/page-event-popup.spec.ts "PO-001 should work @smoke".
    name: "a popup's opener is the page that opened it, and its own popup's is that popup",
    path: '/opener.html',
    source: `const reachedPage = []
    const onPopup = (popup) => reachedPage.push(path(popup.url()))
    page.on('popup', onPopup)
    const [first] = await Promise.all([page.waitForEvent('popup'), page.click('#open')])
    const [second] = await Promise.all([first.waitForEvent('popup'), first.click('#open')])
    page.off('popup', onPopup)
    const openers = {
      first: (await first.opener()) === page,
      second: (await second.opener()) === first,
    }
    const urls = [path(first.url()), path(second.url())]
    await second.close()
    await first.close()
    return { reachedPage, openers, urls }`,
    expected: {
      reachedPage: ['/popup.html?open'],
      openers: { first: true, second: true },
      urls: ['/popup.html?open', '/popup.html?from-popup'],
    },
    probe: 'window.__opened',
    effect: 1,
  },
  {
    // browsercontext-page-event.spec.ts "BC-069 should have an opener" (`await page.opener()`
    // is null for a page of context.newPage()).
    name: "the run's page and a new page have no opener; the new page's popup names it",
    path: '/opener.html',
    source: `const other = await page.context().newPage()
    const none = [await page.opener(), await other.opener()]
    await other.goto(page.url())
    const [popup] = await Promise.all([other.waitForEvent('popup'), other.click('#open')])
    const result = { none, afterGoto: await other.opener(), popup: (await popup.opener()) === other }
    await popup.close()
    await other.close()
    return result`,
    expected: {
      none: [null, null],
      afterGoto: null,
      popup: true,
    },
    // The popup was the new page's, not this one's.
    probe: 'window.__opened',
    effect: 0,
  },
  {
    // tests/library/page-close.spec.ts "PA-047 should return null if parent page has been
    // closed".
    name: "once a popup's opener has closed, its opener() is null",
    path: '/opener.html',
    source: `const [first] = await Promise.all([page.waitForEvent('popup'), page.click('#open')])
    const [second] = await Promise.all([first.waitForEvent('popup'), first.click('#open')])
    const whileOpen = (await second.opener()) === first
    await first.close()
    const result = {
      whileOpen,
      afterClose: await second.opener(),
      closedNamesItsOpener: (await first.opener()) === page,
    }
    await second.close()
    return result`,
    expected: {
      whileOpen: true,
      afterClose: null,
      closedNamesItsOpener: true,
    },
    probe: 'window.__opened',
    effect: 1,
  },
]

test('opener() names the same Pages through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/opener.html': fixture('opener.html'),
      '/popup.html': fixture('popup.html'),
    },
    cases,
  })
})
