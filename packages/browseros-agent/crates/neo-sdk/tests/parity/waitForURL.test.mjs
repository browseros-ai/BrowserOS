// page.waitForURL through the pinned Playwright and through Neo, each case on fresh tabs of
// tests/pages/wait-for-url.html: the current URL matching as an exact URL, a glob, a RegExp,
// `''`, a URL whose scheme needs normalizing and one without its trailing slash; future
// navigations a click starts (a new document, a hash change, history.pushState); `waitUntil`
// commit, domcontentloaded and load on a slow page; a timeout; and calls refused at once.
// Compared: what each call settled to (`undefined` or its error's kind, `waits.mjs` `settle`),
// the URL and title the script read afterwards. Never wording, call logs or timing.
//
// A predicate function sees the current URL, then each navigation's, as a `URL`; a navigation
// that fails before the URL matched rejects the wait.
//
// `waitUntil: 'networkidle'` resolves once the matched page is quiet, in both (neo-runtime
// `network_idle.rs`).
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, waits.mjs).

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import {
  assertDecoysUntouched,
  compareFrameCases,
  FRAME_TREE,
  failed,
} from './frames.mjs'
import { skip } from './harness.mjs'
import { withBoth } from './waits.mjs'

const page = (file) =>
  readFileSync(new URL(`../pages/${file}`, import.meta.url), 'utf8')
const RESOLVED = 'undefined'
const ERROR = { error: 'error' }
/** `/slow` (the slow page's image) answers this late, so its load trails its commit. */
const SLOW_MS = 3000
/** The script's own path of `page.url()`: both engines serve one origin, but only the path is
 * the case's. */
const PATH = `page.url().replace(/^[a-z]+:\\/\\/[^/]+/, '')`

const cases = [
  {
    name: 'the current URL already matches',
    source: `const url = page.url()
    return {
      exact: await settle(page.waitForURL(url)),
      glob: await settle(page.waitForURL('**/wait-for-url.html')),
      regexp: await settle(page.waitForURL(/wait-for-url\\.html$/)),
      everything: await settle(page.waitForURL('')),
      upperCaseScheme: await settle(page.waitForURL(url.replace('http:', 'HTTP:'))),
      commit: await settle(page.waitForURL(url, { waitUntil: 'commit' })),
      domcontentloaded: await settle(page.waitForURL(url, { waitUntil: 'domcontentloaded' })),
    }`,
    expected: {
      exact: RESOLVED,
      glob: RESOLVED,
      regexp: RESOLVED,
      everything: RESOLVED,
      upperCaseScheme: RESOLVED,
      commit: RESOLVED,
      domcontentloaded: RESOLVED,
    },
  },
  {
    name: 'a click navigates to a matching URL',
    source: `const waited = settle(page.waitForURL('**/done.html'))
    await page.click('#to-done')
    return { waited: await waited, path: ${PATH}, title: await page.title() }`,
    expected: { waited: RESOLVED, path: '/done.html', title: '/done.html' },
  },
  {
    name: 'an origin without its trailing slash matches the root',
    source: `const origin = page.url().replace(/\\/wait-for-url\\.html$/, '')
    const waited = settle(page.waitForURL(origin))
    await page.click('#to-root')
    return { waited: await waited, path: ${PATH} }`,
    expected: { waited: RESOLVED, path: '/' },
  },
  {
    name: 'a hash change matches',
    source: `const waited = settle(page.waitForURL(/#section$/))
    await page.click('#to-hash')
    return { waited: await waited, path: ${PATH} }`,
    expected: { waited: RESOLVED, path: '/wait-for-url.html#section' },
  },
  {
    name: 'history.pushState matches',
    source: `const waited = settle(page.waitForURL('**/pushed'))
    await page.click('#push')
    return { waited: await waited, path: ${PATH} }`,
    expected: { waited: RESOLVED, path: '/pushed' },
  },
  {
    name: 'waitUntil commit, domcontentloaded and load on a slow page',
    source: `const committed = page
      .waitForURL('**/slow-load.html', { waitUntil: 'commit' })
      .then(() => page.title())
    await page.click('#to-slow', { noWaitAfter: true })
    const titleAtCommit = await committed
    await page.waitForURL('**/slow-load.html', { waitUntil: 'domcontentloaded' })
    const titleAtDomContentLoaded = await page.title()
    const load = await settle(page.waitForURL('**/slow-load.html'))
    return {
      loadedAtCommit: titleAtCommit === 'load',
      titleAtDomContentLoaded,
      load,
      titleAtLoad: await page.title(),
    }`,
    expected: {
      loadedAtCommit: false,
      titleAtDomContentLoaded: 'domcontentloaded',
      load: RESOLVED,
      titleAtLoad: 'load',
    },
  },
  {
    name: 'a URL never reached times out',
    source: `return {
      glob: await settle(page.waitForURL('**/never.html', { timeout: 300 })),
      regexp: await settle(page.waitForURL(/never/, { timeout: 300 })),
      // Only the primitive '' matches everything; an empty String object is the RegExp /^$/.
      emptyStringObject: await settle(page.waitForURL(new String(''), { timeout: 300 })),
    }`,
    expected: {
      glob: { error: 'timeout' },
      regexp: { error: 'timeout' },
      emptyStringObject: { error: 'timeout' },
    },
  },
  {
    // The 5 s timeout only bounds a regression: each of these fails before it waits.
    name: 'refused at once',
    source: `return {
      notAUrlMatch: await settle(page.waitForURL(42)),
      invalidGlob: await settle(page.waitForURL('**/{a', { timeout: 5000 })),
      waitUntil: await settle(page.waitForURL('**', { waitUntil: 'idle' })),
    }`,
    expected: { notAUrlMatch: ERROR, invalidGlob: ERROR, waitUntil: ERROR },
  },
  {
    name: 'a predicate function sees the current URL, then each navigation',
    source: `const seen = []
    const current = await settle(page.waitForURL((url) => url.pathname.endsWith('/wait-for-url.html')))
    const waited = settle(page.waitForURL((url) => {
      seen.push(url.pathname + url.hash)
      return url.pathname === '/done.html'
    }))
    await page.click('#push')
    await page.click('#to-done')
    return { current, waited: await waited, seen, path: ${PATH} }`,
    expected: {
      current: RESOLVED,
      waited: RESOLVED,
      seen: ['/wait-for-url.html', '/pushed', '/done.html'],
      path: '/done.html',
    },
  },
  {
    // The independent review's regression (review-review-b1.md), after the current-URL check.
    name: 'a later navigation the predicate accepts wins while an earlier answer is pending',
    source: `const waited = settle(page.waitForURL((url) =>
      url.pathname === '/first' ? new Promise(() => {}) : url.pathname === '/goal', { timeout: 1800 }))
    await page.click('#chain')
    return { waited: await waited, path: ${PATH} }`,
    expected: { waited: RESOLVED, path: '/goal' },
  },
  {
    name: 'a navigation that fails before the URL matched rejects the wait',
    source: `const waited = settle(page.waitForURL('**/never.html', { timeout: 10000 }))
    await page.click('#to-unsafe')
    return { waited: await waited }`,
    expected: { waited: ERROR },
  },
  {
    name: 'networkidle resolves once the matched page is quiet',
    source: `return {
      networkidle: await settle(
        page.waitForURL('**/wait-for-url.html', { waitUntil: 'networkidle', timeout: 5000 }),
      ),
    }`,
    expected: { networkidle: RESOLVED },
  },
]

test('page.waitForURL settles the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  const pages = {
    '/wait-for-url.html': page('wait-for-url.html'),
    '/slow-load.html': page('slow-load.html'),
  }
  await withBoth(t, { pages, slowMs: SLOW_MS }, async (run) => {
    for (const c of cases) {
      await t.test(c.name, async () => {
        const { playwright, neo } = await run({
          ...c,
          path: '/wait-for-url.html',
        })
        assert.deepEqual(playwright.result, { value: c.expected }, 'Playwright')
        if (c.neoExpected)
          assert.deepEqual(neo.result, { value: c.neoExpected }, 'Neo limit')
        else
          assert.deepEqual(neo.result, playwright.result, 'Neo vs Playwright')
      })
    }
  })
})

/**
 * waitForURL on a child Frame (page.frame, page.frames()), on tabs of frame-top.html
 * (frames.mjs): the frame's own URL, for the same-origin frame (a glob, a predicate) and the
 * cross-site, out-of-process one (a RegExp, a predicate): already matching, and matched by a
 * navigation a control starts
 * in the frame, which leaves the page's own URL alone (a page wait for it times out). A wait on
 * a frame whose iframe is removed rejects, and it was still pending when the iframe went. The
 * script also waits for each new document through a frame locator, so the page ends the same
 * whether or not child-frame waits are served.
 */
const frameCases = [
  {
    name: 'a child frame waits for its own navigation',
    source: `const where = (url) => url.replace(/^http:\\/\\/([^:/]+):\\d+/, '$1')
    const same = page.frame('same')
    const cross = page.frames().find((frame) => frame.name() === 'cross')
    const already = await settle(() => same.waitForURL('**/frame-child.html?who=same'))
    const sameNavigated = settle(() => same.waitForURL('**/frame-child.html?who=same&gen=2'))
    const samePredicate = settle(() => same.waitForURL((url) => url.searchParams.get('gen') === '2'))
    await page.click('#reload-same')
    const crossNavigated = settle(() => cross.waitForURL(/who=cross.*&gen=2$/))
    const crossPredicate = settle(() => cross.waitForURL((url) => url.searchParams.get('gen') === '2'))
    await page.click('#reload-cross')
    const waits = {
      already,
      same: await sameNavigated,
      samePredicate: await samePredicate,
      cross: await crossNavigated,
      crossPredicate: await crossPredicate,
    }
    await page.frameLocator('#same').getByText('gen 2').waitFor()
    await page.frameLocator('#login-iframe').getByText('gen 2').waitFor()
    return {
      ...waits,
      urls: [where(same.url()), where(cross.url())],
      pageUrl: where(page.url()),
      pageWait: await settle(() => page.waitForURL('**/frame-child.html?who=same&gen=2', { timeout: 300 })),
    }`,
    expected: {
      already: RESOLVED,
      same: RESOLVED,
      samePredicate: RESOLVED,
      cross: RESOLVED,
      crossPredicate: RESOLVED,
      urls: [
        '127.0.0.1/frame-child.html?who=same&gen=2',
        'localhost/frame-child.html?who=cross&kids=grandchild%40127.0.0.1&gen=2',
      ],
      pageUrl: '127.0.0.1/frame-top.html',
      pageWait: failed('timeout'),
    },
    effect: (effect) => {
      const { same, cross } = assertDecoysUntouched(effect, [
        'top',
        'same',
        'cross',
      ])
      assert.deepEqual([same.gen, cross.gen], [2, 2])
    },
  },
  {
    name: 'a URL wait on a child frame rejects when its iframe is removed',
    source: `const removedWhileWaiting = async (frame, remove) => {
      let settled = false
      const waiting = settle(() => frame.waitForURL('**/never.html')).then((result) => {
        settled = true
        return result
      })
      await page.title()
      const pendingAtRemoval = !settled
      await page.click(remove)
      return { pendingAtRemoval, removed: await waiting, detached: frame.isDetached() }
    }
    return {
      same: await removedWhileWaiting(page.frame('same'), '#remove-same'),
      cross: await removedWhileWaiting(page.frame('cross'), '#remove-cross'),
    }`,
    expected: {
      same: {
        pendingAtRemoval: true,
        removed: failed('error'),
        detached: true,
      },
      cross: {
        pendingAtRemoval: true,
        removed: failed('error'),
        detached: true,
      },
    },
    effect: (effect) => {
      const gone = ['same', 'cross', 'grandchild']
      const frames = Object.keys(FRAME_TREE).filter(
        (who) => !gone.includes(who),
      )
      assertDecoysUntouched(effect, ['top'], frames)
    },
  },
]

test('waitForURL on child frames settles the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
