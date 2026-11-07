// context.on('page') through the pinned Playwright and through Neo, on fresh
// tests/pages/opener.html tabs (events.mjs): a popup reaches the context's 'page' listeners and
// then its opener's 'popup' ones as one Page (client/browserContext.ts `_onPage`), already at its
// URL and listed in context.pages(); a context.newPage() page reaches 'page' at about:blank
// before newPage resolves, as the Page it resolves with, and fires no 'popup'; a popup opened
// with no URL or with about:blank is reported at about:blank. Each case closes what it opened
// before it returns: a tab outlives Playwright's disconnect and would still be open through
// Neo's turn. URLs compare by path (`path`).
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    // tests/library/browsercontext-page-event.spec.ts "BC-062 should have url", "BC-069 should
    // have an opener" (context.waitForEvent('page') for a popup); tests/page/page-event-popup.spec.ts
    // "PO-001 should work @smoke".
    name: "a popup reaches the context's 'page', then the page's 'popup', as one Page",
    path: '/opener.html',
    source: `const order = []
    let fromContext
    const onPage = (popup) => {
      fromContext = popup
      order.push(['page', path(popup.url()), page.context().pages().includes(popup)])
    }
    const onPopup = (popup) => order.push(['popup', popup === fromContext])
    page.context().on('page', onPage)
    page.on('popup', onPopup)
    const [waited, popup] = await Promise.all([
      page.context().waitForEvent('page'),
      page.waitForEvent('popup'),
      page.click('#open'),
    ])
    page.context().off('page', onPage)
    page.off('popup', onPopup)
    const identity = { waited: waited === fromContext, popup: popup === fromContext }
    await popup.close()
    return { order, identity, listenersLeft: [page.context().listenerCount('page'), page.listenerCount('popup')] }`,
    expected: {
      order: [
        ['page', '/popup.html?open', true],
        ['popup', true],
      ],
      identity: { waited: true, popup: true },
      listenersLeft: [0, 0],
    },
    probe: 'window.__opened',
    effect: 1,
  },
  {
    // browsercontext-page-event.spec.ts "BC-067 should report initialized pages", "BC-070
    // should fire page lifecycle events" ('CREATED: about:blank').
    name: "context.newPage() reaches 'page' at about:blank before it resolves, and no 'popup'",
    path: '/opener.html',
    source: `const seen = []
    const onPage = (other) => seen.push(['page', other.url(), other.isClosed(), page.context().pages().includes(other)])
    const onPopup = (popup) => seen.push(['popup', popup.url()])
    page.context().on('page', onPage)
    page.on('popup', onPopup)
    const waiting = page.context().waitForEvent('page')
    const other = await page.context().newPage()
    const atResolve = [...seen]
    const waited = await waiting
    page.context().off('page', onPage)
    page.off('popup', onPopup)
    const result = { atResolve, sameObject: waited === other, another: other !== page, url: other.url() }
    await other.close()
    return result`,
    expected: {
      atResolve: [['page', 'about:blank', false, true]],
      sameObject: true,
      another: true,
      url: 'about:blank',
    },
    probe: 'window.__opened',
    effect: 0,
  },
  {
    // browsercontext-page-event.spec.ts "BC-064 should have about:blank url with
    // domcontentloaded", "BC-065 should have about:blank for empty url with domcontentloaded",
    // "BC-067 should report initialized pages"; page-event-popup.spec.ts "PO-006 should work
    // with empty url".
    name: 'a popup opened with no URL or with about:blank is reported at about:blank',
    path: '/opener.html',
    source: `const popups = {}
    for (const selector of ['#empty', '#blank']) {
      const [popup, fromPopup] = await Promise.all([
        page.context().waitForEvent('page'),
        page.waitForEvent('popup'),
        page.click(selector),
      ])
      const atEvent = popup.url()
      await popup.waitForLoadState('domcontentloaded')
      popups[selector] = {
        urls: [atEvent, popup.url()],
        samePage: popup === fromPopup,
        opener: (await popup.opener()) === page,
      }
      await popup.close()
    }
    return popups`,
    expected: {
      '#empty': {
        urls: ['about:blank', 'about:blank'],
        samePage: true,
        opener: true,
      },
      '#blank': {
        urls: ['about:blank', 'about:blank'],
        samePage: true,
        opener: true,
      },
    },
    probe: 'window.__opened',
    effect: 2,
  },
]

test("context 'page' events report the same Pages through Neo and Playwright", {
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
