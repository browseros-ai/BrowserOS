// page.waitForLoadState through the pinned Playwright and through Neo, each case on fresh tabs:
// every state of a loaded page; a page whose image answers late, which reaches commit and
// domcontentloaded long before load (a short timeout on load times out meanwhile); a page whose
// parser-blocking script answers late, which holds domcontentloaded back (a short timeout on it
// times out meanwhile); and states refused at once. Compared: what each call settled to (`undefined` or its error's kind,
// `waits.mjs` `settle`) and the title the page's own DOMContentLoaded and load handlers set.
// Never wording, call logs or timing.
//
// `networkidle` resolves on a quiet loaded page in both (neo-runtime `network_idle.rs`).
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

const RESOLVED = 'undefined'
const ERROR = { error: 'error' }
/** `/slow` answers this late: slow-load.html's image, so its load trails its commit, and
 * dcl-blocked.html's script, so its domcontentloaded does. */
const SLOW_MS = 3000

const cases = [
  {
    name: 'a loaded page has every state',
    path: '/loaded.html',
    source: `return {
      load: await settle(page.waitForLoadState()),
      domcontentloaded: await settle(page.waitForLoadState('domcontentloaded')),
      commit: await settle(page.waitForLoadState('commit')),
      frame: await settle(page.mainFrame().waitForLoadState('load')),
    }`,
    expected: {
      load: RESOLVED,
      domcontentloaded: RESOLVED,
      commit: RESOLVED,
      frame: RESOLVED,
    },
  },
  {
    // The script starts once the page has committed, seconds before its image answers.
    name: 'a loading page reaches domcontentloaded, then load',
    path: '/slow-load.html',
    loaded: false,
    source: `const early = await settle(page.waitForLoadState('load', { timeout: 300 }))
    const commit = await settle(page.waitForLoadState('commit'))
    const domcontentloaded = await settle(page.waitForLoadState('domcontentloaded'))
    const titleAtDomContentLoaded = await page.title()
    const load = await settle(page.waitForLoadState())
    return {
      early,
      commit,
      domcontentloaded,
      titleAtDomContentLoaded,
      load,
      titleAtLoad: await page.title(),
    }`,
    expected: {
      early: { error: 'timeout' },
      commit: RESOLVED,
      domcontentloaded: RESOLVED,
      titleAtDomContentLoaded: 'domcontentloaded',
      load: RESOLVED,
      titleAtLoad: 'load',
    },
  },
  {
    // The script starts once the page has committed, while its parser waits for `/slow`. The
    // title before is the document's own; DOMContentLoaded's handler renames it.
    name: 'a page held before domcontentloaded reaches it later',
    path: '/dcl-blocked.html',
    loaded: false,
    source: `const early = await settle(page.waitForLoadState('domcontentloaded', { timeout: 300 }))
    const titleBefore = await page.title()
    const domcontentloaded = await settle(page.waitForLoadState('domcontentloaded'))
    return {
      early,
      titleBefore,
      domcontentloaded,
      titleAtDomContentLoaded: await page.title(),
    }`,
    expected: {
      early: { error: 'timeout' },
      titleBefore: 'committed',
      domcontentloaded: RESOLVED,
      titleAtDomContentLoaded: 'domcontentloaded',
    },
  },
  {
    name: 'refused at once',
    path: '/loaded.html',
    source: `return {
      unknown: await settle(page.waitForLoadState('idle')),
      nullState: await settle(page.waitForLoadState(null)),
    }`,
    expected: { unknown: ERROR, nullState: ERROR },
  },
  {
    // tests/page/page-network-idle.spec.ts "NE-070 should navigate to empty page with
    // networkidle", for a loaded page.
    name: 'networkidle resolves on a quiet loaded page',
    path: '/loaded.html',
    source: `return {
      networkidle: await settle(page.waitForLoadState('networkidle', { timeout: 5000 })),
    }`,
    expected: { networkidle: RESOLVED },
  },
]

test('page.waitForLoadState settles the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  const page = (file) =>
    readFileSync(new URL(`../pages/${file}`, import.meta.url), 'utf8')
  const pages = {
    '/slow-load.html': page('slow-load.html'),
    '/dcl-blocked.html': page('dcl-blocked.html'),
    // A script for dcl-blocked.html; slow-load.html's image only needs the response to end.
    '/slow': "document.documentElement.dataset.slow = 'answered'",
  }
  await withBoth(t, { pages, slowMs: SLOW_MS }, async (run) => {
    for (const c of cases) {
      await t.test(c.name, async () => {
        const { playwright, neo } = await run(c)
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
 * waitForLoadState on a child Frame (page.frame, page.frames()), on tabs of frame-top.html
 * (frames.mjs): the frame's own document, for the same-origin frame and the cross-site,
 * out-of-process one. A control loads a new document into the frame whose image answers
 * SLOW_MS late, so a short wait on its load times out while domcontentloaded resolves, and load
 * resolves later. A wait on a frame whose iframe is removed rejects, and it was still pending
 * when the iframe went (a wait refused at once would have settled before). The scripts wait for
 * each new document through a frame locator too, so the page ends the same whether or not
 * child-frame waits are served.
 */
const frameCases = [
  {
    name: "a child frame waits for its own new document's load states",
    source: `const states = async (frame, control, iframe) => {
      const before = await settle(() => frame.waitForLoadState())
      await page.click(control)
      await page.frameLocator(iframe).getByText('gen 2').waitFor()
      return {
        before,
        early: await settle(() => frame.waitForLoadState('load', { timeout: 300 })),
        domcontentloaded: await settle(() => frame.waitForLoadState('domcontentloaded')),
        load: await settle(() => frame.waitForLoadState()),
      }
    }
    return {
      same: await states(page.frame('same'), '#slow-same', '#same'),
      cross: await states(page.frames().find((frame) => frame.name() === 'cross'), '#slow-cross', '#login-iframe'),
    }`,
    expected: {
      same: {
        before: RESOLVED,
        early: failed('timeout'),
        domcontentloaded: RESOLVED,
        load: RESOLVED,
      },
      cross: {
        before: RESOLVED,
        early: failed('timeout'),
        domcontentloaded: RESOLVED,
        load: RESOLVED,
      },
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
    name: 'a load wait on a child frame rejects when its iframe is removed',
    source: `const removedWhileWaiting = async (frame, control, iframe, remove) => {
      await page.click(control)
      await page.frameLocator(iframe).getByText('gen 2').waitFor()
      let settled = false
      const waiting = settle(() => frame.waitForLoadState()).then((result) => {
        settled = true
        return result
      })
      await page.title()
      const pendingAtRemoval = !settled
      await page.click(remove)
      return { pendingAtRemoval, removed: await waiting, detached: frame.isDetached() }
    }
    return {
      same: await removedWhileWaiting(page.frame('same'), '#slow-same', '#same', '#remove-same'),
      cross: await removedWhileWaiting(page.frame('cross'), '#slow-cross', '#login-iframe', '#remove-cross'),
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

test('waitForLoadState on child frames settles the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases, { slowMs: SLOW_MS })
})
