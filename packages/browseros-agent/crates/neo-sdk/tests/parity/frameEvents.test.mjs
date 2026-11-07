// 'frameattached', 'framenavigated' and 'framedetached' on the page and on its context through
// the pinned Playwright and through Neo, on fresh tests/pages/frame-events.html tabs
// (events.mjs): an iframe the page adds reaches the page's listeners and then the context's as
// one Frame (client/page.ts `_onFrameAttached`, `_onFrameDetached`, client/frame.ts's
// 'navigated' handler): attached with no name or URL yet, then navigated, named and at its URL,
// for each document it loads, then detached; until then page.frame() finds that same Frame and
// its parent is the main frame. Leaving the page detaches its child frames and then navigates
// the main frame, which stays the same Frame and is never attached or detached. A cross-site
// iframe, which Chromium runs in a process of its own, goes the same way once the script has
// reached it.
//
// BrowserOS's own content script (claw-app's takeover.content.ts) adds and removes two
// about:blank iframes on some loads and not others, at a moment of its own, so the cases follow
// their frames by identity and never count frames.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

/**
 * Records every frame event of the page and of its context in `events`, with the Frame and
 * what it says at that moment; `stop()` takes the listeners off.
 */
const LISTEN = `const events = []
    const listeners = []
    for (const [who, emitter] of [['page', page], ['context', page.context()]]) {
      for (const type of ['frameattached', 'framenavigated', 'framedetached']) {
        const listener = (frame) => {
          events.push({ frame, seen: [who, type, frame.name(), path(frame.url()), frame.isDetached()] })
        }
        emitter.on(type, listener)
        listeners.push([emitter, type, listener])
      }
    }
    const stop = () => {
      for (const [emitter, type, listener] of listeners) emitter.off(type, listener)
    }
    `

/** Adds the iframe `name` (same or cross), moves it to its next document, removes it. */
const lifecycle = (name) => `${LISTEN}const [frame] = await Promise.all([
      page.waitForEvent('framenavigated', (navigated) => path(navigated.url()) === '/${name}-1'),
      page.click('#add-${name}'),
    ])
    const added = {
      byName: page.frame({ name: '${name}' }) === frame,
      byUrl: page.frame({ url: /\\/${name}-1$/ }) === frame,
      parent: frame.parentFrame() === page.mainFrame(),
      page: frame.page() === page,
      listed: [page.frames().includes(frame), page.mainFrame().childFrames().includes(frame)],
      crossSite: /^http:\\/\\/localhost:\\d+\\//.test(frame.url()),
    }
    const [renavigated] = await Promise.all([
      page.waitForEvent('framenavigated', (navigated) => path(navigated.url()) === '/${name}-2'),
      page.click('#navigate-${name}'),
    ])
    await frame.waitForLoadState()
    const title = await frame.title()
    const [detached] = await Promise.all([
      page.waitForEvent('framedetached', (gone) => gone.name() === '${name}'),
      page.click('#remove-${name}'),
    ])
    stop()
    return {
      seen: events.filter((event) => event.frame === frame).map((event) => event.seen),
      added,
      title,
      sameFrame: [renavigated === frame, detached === frame],
      removed: [frame.isDetached(), page.frames().includes(frame), page.mainFrame().childFrames().includes(frame), page.frame({ name: '${name}' })],
    }`

/** What `lifecycle(name)` sees of its frame: a page event, then the context's, every time. */
const lifecycleSeen = (name) =>
  [
    ['frameattached', '', '', false],
    ['framenavigated', name, `/${name}-1`, false],
    ['framenavigated', name, `/${name}-2`, false],
    ['framedetached', name, `/${name}-2`, true],
  ].flatMap((event) => [
    ['page', ...event],
    ['context', ...event],
  ])

const cases = [
  {
    // tests/page/frame-hierarchy.spec.ts "FR-025 should send events when frames are
    // manipulated dynamically", "FR-032 should report frame.name()", "FR-033 should report
    // frame.parent()", "FR-036 should return frame.page()"; tests/library/browsercontext-events.spec.ts
    // "BC-048 frameattached event should work @smoke", "BC-049 framedetached event should work
    // @smoke".
    name: "a same-origin iframe's attach, navigations and detach reach page then context",
    path: '/frame-events.html',
    source: lifecycle('same'),
    // At 'frameattached' a frame has no name or URL: CDP's Page.frameAttached carries neither,
    // and its first commit brings both.
    expected: {
      seen: lifecycleSeen('same'),
      added: {
        byName: true,
        byUrl: true,
        parent: true,
        page: true,
        listed: [true, true],
        crossSite: false,
      },
      title: '/same-2',
      sameFrame: [true, true],
      removed: [true, false, false, null],
    },
    probe: "[...document.querySelectorAll('iframe')].map((frame) => frame.id)",
    effect: ['first'],
  },
  {
    // frame-hierarchy.spec.ts "FR-029 should detach child frames on navigation", "FR-028
    // should not send attach/detach events for main frame", "FR-027 should persist mainFrame on
    // cross-process navigation" (here a same-origin one); browsercontext-events.spec.ts
    // "BC-046 framenavigated event should work @smoke".
    name: 'leaving the page detaches its child frames, then navigates the same main frame',
    path: '/frame-events.html',
    source: `const main = page.mainFrame()
    const first = page.frame({ name: 'first' })
    const before = [path(first.url()), first.parentFrame() === main]
    ${LISTEN}const [navigated] = await Promise.all([
      page.context().waitForEvent('framenavigated', (frame) => frame.page() === page && frame.parentFrame() === null),
      page.click('#leave'),
    ])
    await page.waitForLoadState()
    stop()
    const known = (frame) => (frame === main ? 'main' : frame === first ? 'first' : null)
    return {
      before,
      seen: events.filter((event) => known(event.frame)).map((event) => [known(event.frame), ...event.seen]),
      navigated: [navigated === main, page.mainFrame() === main, path(navigated.url())],
      first: [first.isDetached(), page.frames().includes(first), main.childFrames().includes(first)],
    }`,
    // Each child is gone before the main frame's commit is reported (server/frames.ts
    // `frameCommittedNewDocumentNavigation`).
    expected: {
      before: ['/first-1', true],
      seen: [
        ['first', 'page', 'framedetached', 'first', '/first-1', true],
        ['first', 'context', 'framedetached', 'first', '/first-1', true],
        ['main', 'page', 'framenavigated', '', '/left', false],
        ['main', 'context', 'framenavigated', '', '/left', false],
      ],
      navigated: [true, true, '/left'],
      first: [true, false, false],
    },
    probe: 'location.pathname',
    effect: '/left',
  },
  {
    // Cross-site: the iframe loads from http://localhost at the fixture server's port, another
    // site than the page's 127.0.0.1, so Chromium runs it out of process (an OOPIF, a CDP target
    // of its own). frame-hierarchy.spec.ts "FR-025 should send events when frames are
    // manipulated dynamically", for such a frame once the script has reached it: Neo attaches
    // an out-of-process frame when the run first reads into it (here page.frame()), so the
    // frame's own first commit, before that, is not reported (neo-runtime/src/frame_events.rs,
    // Deviation); its later navigations and its removal are, as in Playwright.
    name: 'a cross-site iframe the script has reached navigates and detaches the same',
    path: '/frame-events.html',
    source: `await page.click('#add-cross')
    let frame
    while (!(frame = page.frame({ name: 'cross' })) || path(frame.url()) !== '/cross-1')
      await new Promise((resolve) => setTimeout(resolve, 50))
    await frame.waitForLoadState()
    const crossSite = /^http:\\/\\/localhost:\\d+\\//.test(frame.url())
    ${LISTEN}const [renavigated] = await Promise.all([
      page.waitForEvent('framenavigated', (navigated) => path(navigated.url()) === '/cross-2'),
      page.click('#navigate-cross'),
    ])
    await frame.waitForLoadState()
    const title = await frame.title()
    const [detached] = await Promise.all([
      page.waitForEvent('framedetached', (gone) => gone.name() === 'cross'),
      page.click('#remove-cross'),
    ])
    stop()
    return {
      crossSite,
      seen: events.filter((event) => event.frame === frame).map((event) => event.seen),
      title,
      sameFrame: [renavigated === frame, detached === frame],
      removed: [frame.isDetached(), page.frames().includes(frame), page.frame({ name: 'cross' })],
    }`,
    expected: {
      crossSite: true,
      seen: [
        ['framenavigated', 'cross', '/cross-2', false],
        ['framedetached', 'cross', '/cross-2', true],
      ].flatMap((event) => [
        ['page', ...event],
        ['context', ...event],
      ]),
      title: '/cross-2',
      sameFrame: [true, true],
      removed: [true, false, null],
    },
    probe: "[...document.querySelectorAll('iframe')].map((frame) => frame.id)",
    effect: ['first'],
  },
]

test('frame events report the same Frames in the same order through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: { '/frame-events.html': fixture('frame-events.html') },
    cases,
  })
})
