// page.on('close') and context.on('pageclose') through the pinned Playwright and through Neo, on
// fresh tests/pages/opener.html tabs (events.mjs): page.close() on a context.newPage() page and
// on a popup fires the page's 'close' and then the context's 'pageclose', each with the page,
// already closed and gone from context.pages(), and only then resolves (client/page.ts
// `_onClose`; Playwright's server answers close() once its page has closed); a popup that
// closes itself (window.close from a click in it) fires the same two, its click resolves, and
// its opener stays open. Each case closes what it opened before it returns: a tab outlives
// Playwright's disconnect and would still be open through Neo's turn.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    // tests/library/page-close.spec.ts "PA-043 should set the page close state", "PA-044 should
    // pass page to close event"; tests/library/browsercontext-events.spec.ts "BC-047 pageclose
    // event should work @smoke"; tests/library/browsercontext-page-event.spec.ts "BC-066 should
    // report when a new page is created and closed".
    name: "close() fires 'close' then 'pageclose', then resolves, for a new page and a popup",
    path: '/opener.html',
    source: `const order = []
    let closing
    const onPageClose = (closed) => order.push(['pageclose', closed === closing, page.context().pages().includes(closed)])
    page.context().on('pageclose', onPageClose)
    const close = async (target) => {
      closing = target
      target.once('close', (closed) => order.push(['close', closed === target, target.isClosed()]))
      const resolved = await target.close()
      order.push(['close() resolved', resolved === undefined])
      return order.splice(0)
    }
    const other = await page.context().newPage()
    const newPage = await close(other)
    const [popup] = await Promise.all([page.waitForEvent('popup'), page.click('#open')])
    const popupClose = await close(popup)
    page.context().off('pageclose', onPageClose)
    return {
      newPage,
      popup: popupClose,
      closed: [other.isClosed(), popup.isClosed()],
      listed: [page.context().pages().includes(other), page.context().pages().includes(popup)],
      pageOpen: !page.isClosed(),
    }`,
    expected: {
      newPage: [
        ['close', true, true],
        ['pageclose', true, false],
        ['close() resolved', true],
      ],
      popup: [
        ['close', true, true],
        ['pageclose', true, false],
        ['close() resolved', true],
      ],
      closed: [true, true],
      listed: [false, false],
      pageOpen: true,
    },
    probe: 'window.__opened',
    effect: 1,
  },
  {
    // tests/library/popup.spec.ts "PO-028 should not throw when click closes popup";
    // browsercontext-events.spec.ts "BC-047 pageclose event should work @smoke".
    name: "a popup that closes itself fires 'close' then 'pageclose', and its click resolves",
    path: '/opener.html',
    source: `const [popup] = await Promise.all([page.waitForEvent('popup'), page.click('#open')])
    await popup.waitForLoadState()
    const order = []
    popup.once('close', (closed) => order.push(['close', closed === popup, popup.isClosed()]))
    const onPageClose = (closed) => {
      if (closed === popup) order.push(['pageclose', page.context().pages().includes(closed)])
    }
    page.context().on('pageclose', onPageClose)
    let openerClosed = false
    const onOpenerClose = () => {
      openerClosed = true
    }
    page.on('close', onOpenerClose)
    // The click's own outcome is left out here: Neo's click rejects when its page closes under
    // it, where Playwright's resolves (the Neo-only case below pins it).
    const [closed, pageClosed] = await Promise.all([
      popup.waitForEvent('close'),
      page.context().waitForEvent('pageclose'),
      popup.click('#close').catch(() => {}),
    ])
    page.context().off('pageclose', onPageClose)
    page.off('close', onOpenerClose)
    return {
      order,
      waited: [closed === popup, pageClosed === popup],
      isClosed: popup.isClosed(),
      listed: page.context().pages().includes(popup),
      openerClosed,
    }`,
    expected: {
      order: [
        ['close', true, true],
        ['pageclose', false],
      ],
      waited: [true, true],
      isClosed: true,
      listed: false,
      openerClosed: false,
    },
    probe: 'window.__opened',
    effect: 1,
  },
  {
    // Neo only, a recorded gap of Neo's click (neo-runtime/src/click.rs, `PressReport::Failed`
    // with `closed`): a click whose page closes under it rejects with TargetClosedError, where
    // Playwright's resolves (tests/library/popup.spec.ts "PO-028 should not throw when click
    // closes popup"). The close events themselves match Playwright (the case above).
    name: "Neo's click on a button that closes its own popup rejects as closed",
    path: '/opener.html',
    source: `const [popup] = await Promise.all([page.waitForEvent('popup'), page.click('#open')])
    await popup.waitForLoadState()
    const clicked = await settle(popup.click('#close'))
    return { clicked, isClosed: popup.isClosed() }`,
    neo: (outcome) =>
      assert.deepEqual(outcome.value, {
        clicked: { error: 'Error' },
        isClosed: true,
      }),
  },
]

test("'close' and 'pageclose' fire in the same order through Neo and Playwright", {
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
