// page.on('popup') and page.opener() through the pinned Playwright and through Neo, on fresh
// tests/pages/opener.html tabs (events.mjs): a popup the page opens with window.open reaches
// page.waitForEvent('popup') once its first document has committed, so already at its URL, as a
// Page of the page's context, listed there, whose opener() is the page; one opened with
// 'noopener' and one a target=_blank link opens (noopener as well, as Chromium treats such a
// link) reach it the same way, and Playwright names the page their opener too. Each case closes
// its popup before it returns: a tab outlives Playwright's disconnect and would still be open
// through Neo's turn. URLs compare by path (`path`).
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    // tests/page/page-event-popup.spec.ts "PO-001 should work @smoke" (window.open, here with a
    // URL), "PO-010 should work with clicking target=_blank" (`popup.mainFrame().page()`);
    // tests/library/browsercontext-page-event.spec.ts "BC-062 should have url", "BC-069 should
    // have an opener". Playwright reports a popup only once its first document has committed
    // (server/chromium/crPage.ts `_firstNonInitialNavigationCommittedPromise`).
    name: "a window.open popup reaches waitForEvent('popup') at its URL, the page its opener",
    path: '/opener.html',
    source: `const [popup] = await Promise.all([page.waitForEvent('popup'), page.click('#open')])
    const atEvent = [path(popup.url()), popup.isClosed()]
    await popup.waitForLoadState()
    const described = {
      atEvent,
      loaded: [path(popup.url()), await popup.title()],
      another: popup !== page,
      context: popup.context() === page.context(),
      mainFrame: popup.mainFrame().page() === popup,
      listed: page.context().pages().includes(popup),
      opener: (await popup.opener()) === page,
      listenersLeft: page.listenerCount('popup'),
    }
    await popup.close()
    return { ...described, closed: [popup.isClosed(), page.context().pages().includes(popup)] }`,
    expected: {
      atEvent: ['/popup.html?open', false],
      loaded: ['/popup.html?open', 'popup open'],
      another: true,
      context: true,
      mainFrame: true,
      listed: true,
      opener: true,
      listenersLeft: 0,
      closed: [true, false],
    },
    probe: 'window.__opened',
    effect: 1,
  },
  {
    // page-event-popup.spec.ts "PO-009 should work with noopener and url". The spec reads only
    // each window's own `window.opener`, which takes page.evaluate.
    name: "a noopener popup still reaches 'popup', and Playwright names the page its opener",
    path: '/opener.html',
    source: `const [popup] = await Promise.all([page.waitForEvent('popup'), page.click('#noopener')])
    const atEvent = path(popup.url())
    await popup.waitForLoadState()
    const described = {
      atEvent,
      loaded: [path(popup.url()), await popup.title()],
      context: popup.context() === page.context(),
      opener: (await popup.opener()) === page,
    }
    await popup.close()
    return described`,
    // opener() is the page, not null: under noopener Chromium still reports the opening tab as
    // the popup's openerId (only its canAccessOpener is false), and Playwright takes a popup's
    // opener from openerId (server/chromium/crBrowser.ts `_onAttachedToTarget`).
    expected: {
      atEvent: '/popup.html?noopener',
      loaded: ['/popup.html?noopener', 'popup noopener'],
      context: true,
      opener: true,
    },
    probe: 'window.__opened',
    effect: 1,
  },
  {
    // page-event-popup.spec.ts "PO-012 should work with clicking target=_blank and rel=noopener"
    // (Chromium makes a target=_blank link without rel=opener noopener), "PO-010 should work
    // with clicking target=_blank".
    name: "a target=_blank link's tab reaches 'popup' and the page stays where it was",
    path: '/opener.html',
    source: `const [popup] = await Promise.all([page.waitForEvent('popup'), page.click('#link')])
    const atEvent = path(popup.url())
    await popup.waitForLoadState()
    const described = {
      atEvent,
      loaded: [path(popup.url()), await popup.title()],
      mainFrame: popup.mainFrame().page() === popup,
      opener: (await popup.opener()) === page,
      pageUrl: path(page.url()),
    }
    await popup.close()
    return described`,
    // opener() is the page, as for window.open's noopener above.
    expected: {
      atEvent: '/popup.html?link',
      loaded: ['/popup.html?link', 'popup link'],
      mainFrame: true,
      opener: true,
      pageUrl: '/opener.html',
    },
    probe: 'window.__opened',
    effect: 1,
  },
]

test('popups reach the same listeners as the same Pages through Neo and Playwright', {
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
