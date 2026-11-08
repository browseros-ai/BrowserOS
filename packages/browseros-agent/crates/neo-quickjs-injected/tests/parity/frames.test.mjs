// The direct frame API through the pinned Playwright and through Neo, on fresh tabs of
// tests/pages/frame-top.html: page.frames(), mainFrame(), frame(name) and frame({ url }) with a
// RegExp, a glob and an exact URL; each frame's name(), url(), parentFrame(), childFrames(),
// page(), isDetached() and title(); one object per frame across every getter; and what a Frame
// object reads after its frame navigated (the same frame), was replaced by a new iframe, or was
// removed (detached; its calls fail). Compared: values and their types, error categories
// (`frames.mjs` `settle`) and what every frame held afterwards. Never wording or timing.
//
// The first case checks the fixture itself: in both tabs CDP lists the cross-site frames and the
// 127.0.0.1 frame inside one as `iframe` targets (out of process) and no same-process frame.
//
// Known, intended differences, asserted per engine and not counted as parity:
// - `frames()` order (design §4.7). Playwright lists frames as they attached; Neo lists the tree
//   in pre-order, main frame first, a frame's same-process children (in document order) before
//   its out-of-process ones.
// - A call on the Frame of an out-of-process frame that is gone (its iframe replaced). Playwright
//   rejects it as "Target page, context or browser has been closed", the wording of a closed
//   page, because the frame's own CDP session closed; Neo rejects it as "Frame was detached",
//   as for any detached frame, since only the tab's end closes a page (design §3.1, §3.4).
//   A gone same-process frame fails "Frame was detached" on both.
// - `goto` on a child frame (design §4.5): Playwright navigates the frame; Neo refuses it with an
//   `Error` and leaves the frame as it was.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, frames.mjs).

import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  assertDecoysUntouched,
  compareFrameCases,
  FRAME_TREE,
  failed,
  pristine,
  value,
} from './frames.mjs'
import { skip } from './harness.mjs'

/** A frame URL as the scripts report it: the host kept, the port dropped. */
const WHERE = `const where = (url) => url.replace(/^http:\\/\\/([^:/]+):\\d+/, '$1')\n`
const NAME = `const name = (frame) => frame.name() || 'top'\n`
const OUT_OF_PROCESS = [
  '127.0.0.1/frame-child.html?who=grandchild',
  'localhost/frame-child.html?who=cross&kids=grandchild@127.0.0.1',
  'localhost/frame-child.html?who=hidden-cross',
]
const child = (who, extra = '') =>
  `127.0.0.1/frame-child.html?who=${who}${extra}`
const TREE = {
  top: { url: '127.0.0.1/frame-top.html', parent: null },
  same: { url: child('same'), parent: 'top' },
  cross: { url: OUT_OF_PROCESS[1], parent: 'top' },
  grandchild: { url: child('grandchild'), parent: 'cross' },
  chain: { url: child('chain', '&kids=chain-inner'), parent: 'top' },
  'chain-inner': { url: child('chain-inner'), parent: 'chain' },
  'hidden-same': { url: child('hidden-same'), parent: 'top' },
  'hidden-cross': { url: OUT_OF_PROCESS[2], parent: 'top' },
}
/** `TREE` as the first case's script describes each frame. */
const DESCRIBED = Object.fromEntries(
  Object.entries(TREE).map(([who, { url, parent }]) => [
    who,
    {
      url,
      parent,
      children: Object.keys(FRAME_TREE)
        .filter((kid) => FRAME_TREE[kid] === who)
        .sort(),
      detached: false,
      page: true,
    },
  ]),
)
/**
 * Neo's `frames()` order (design §4.7): pre-order, each frame's same-process children in
 * document order, then its out-of-process ones (in either order: CDP lists targets unordered).
 */
const NEO_ORDERS = [
  [
    'top',
    'same',
    'chain',
    'chain-inner',
    'hidden-same',
    'cross',
    'grandchild',
    'hidden-cross',
  ],
  [
    'top',
    'same',
    'chain',
    'chain-inner',
    'hidden-same',
    'hidden-cross',
    'cross',
    'grandchild',
  ],
]

const cases = [
  {
    name: 'the cross-site frames are out of process in both tabs',
    source: `return page.url().endsWith('/frame-top.html')`,
    expected: true,
    effect: (effect) => assertDecoysUntouched(effect, []),
    checkPlaywright: ({ iframes }) =>
      assert.deepEqual(iframes, OUT_OF_PROCESS, "Playwright's tab"),
    checkNeo: ({ iframes }) =>
      assert.deepEqual(iframes, OUT_OF_PROCESS, "Neo's tab"),
  },
  {
    name: 'the frame tree, one object per frame',
    source: `${WHERE}${NAME}const main = page.mainFrame()
    const frames = page.frames()
    const describe = (frame) => ({
      url: where(frame.url()),
      parent: frame.parentFrame() === null ? null : name(frame.parentFrame()),
      children: frame.childFrames().map(name).sort(),
      detached: frame.isDetached(),
      page: frame.page() === page,
    })
    return {
      order: frames.map(name),
      tree: Object.fromEntries(frames.map((frame) => [name(frame), describe(frame)])),
      main: { first: frames[0] === main, name: main.name(), parent: main.parentFrame() },
      identity: {
        again: page.frames().every((frame, i) => frame === frames[i]),
        byName: page.frame('cross') === frames.find((frame) => frame.name() === 'cross'),
        parent: page.frame('grandchild').parentFrame() === page.frame('cross'),
        child: page.frame('chain').childFrames()[0] === page.frame('chain-inner'),
        byUrl: page.frame({ url: /frame-top\\.html$/ }) === main,
      },
      find: {
        regExp: page.frame({ url: /who=grandchild/ })?.name(),
        glob: page.frame({ url: '**/frame-child.html?who=chain&kids=chain-inner' })?.name(),
        exact: page.frame({ url: page.frame('same').url() })?.name(),
        missingName: page.frame('nope'),
        missingUrl: page.frame({ url: /nope/ }),
      },
      titles: await settle(() => Promise.all(['cross', 'grandchild', 'chain-inner'].map((who) => page.frame(who).title()))),
    }`,
    perEngine: ['order'],
    expected: {
      tree: DESCRIBED,
      main: { first: true, name: '', parent: null },
      identity: {
        again: true,
        byName: true,
        parent: true,
        child: true,
        byUrl: true,
      },
      find: {
        regExp: 'grandchild',
        glob: 'chain',
        exact: 'same',
        missingName: null,
        missingUrl: null,
      },
      titles: value(['Frame cross', 'Frame grandchild', 'Frame chain-inner']),
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
    checkPlaywright: ({ result }) => {
      const { order } = result.value
      assert.equal(order[0], 'top', 'Playwright lists the main frame first')
      for (const who of order.slice(1))
        assert.ok(
          order.indexOf(FRAME_TREE[who]) < order.indexOf(who),
          `Playwright lists ${who} after its parent`,
        )
    },
    checkNeo: ({ result }) =>
      assert.ok(
        NEO_ORDERS.some(
          (order) =>
            JSON.stringify(order) === JSON.stringify(result.value.order),
        ),
        `Neo's frames() order, design §4.7: ${result.value.order}`,
      ),
  },
  {
    name: 'a Frame after its frame navigated, was replaced, or was removed',
    source: `${WHERE}${NAME}const same = page.frame('same')
    const cross = page.frame('cross')
    const grandchild = page.frame('grandchild')
    await page.click('#reload-same')
    await page.frameLocator('#same').getByText('gen 2').waitFor()
    const navigated = {
      same: page.frame('same') === same,
      url: where(same.url()),
      detached: same.isDetached(),
      gen: await settle(() => same.textContent('#gen')),
    }
    await page.click('#replace-cross')
    await page.frameLocator('#login-iframe').getByText('gen 2').waitFor()
    const fresh = page.frame('cross')
    const replaced = {
      same: fresh === cross,
      oldDetached: cross.isDetached(),
      oldChildDetached: grandchild.isDetached(),
      url: where(fresh.url()),
      freshDetached: fresh.isDetached(),
      freshGen: await settle(() => fresh.textContent('#gen')),
    }
    const staleOutOfProcess = await settle(() => cross.textContent('#who', { timeout: 2000 }))
    await page.click('#remove-same')
    await page.locator('#same').waitFor({ state: 'detached' })
    const removed = {
      detached: same.isDetached(),
      listed: page.frames().includes(same),
      byName: page.frame('same'),
      children: page.mainFrame().childFrames().map(name).sort(),
      staleRead: await settle(() => same.textContent('#who', { timeout: 2000 })),
      staleFill: await settle(() => same.fill('#email', 'gone', { timeout: 2000 })),
    }
    return { navigated, replaced, removed, staleOutOfProcess }`,
    expected: {
      navigated: {
        same: true,
        url: child('same', '&gen=2'),
        detached: false,
        gen: value('gen 2'),
      },
      replaced: {
        same: false,
        oldDetached: true,
        oldChildDetached: true,
        url: 'localhost/frame-child.html?who=cross&kids=grandchild%40127.0.0.1&gen=2',
        freshDetached: false,
        freshGen: value('gen 2'),
      },
      removed: {
        detached: true,
        listed: false,
        byName: null,
        children: ['chain', 'cross', 'hidden-cross', 'hidden-same'],
        staleRead: failed('error'),
        staleFill: failed('error'),
      },
    },
    perEngine: ['staleOutOfProcess'],
    checkPlaywright: ({ result }) =>
      assert.deepEqual(result.value.staleOutOfProcess, failed('closed')),
    checkNeo: ({ result }) =>
      assert.deepEqual(
        result.value.staleOutOfProcess,
        failed('error'),
        'Neo: Frame was detached, design §3.1',
      ),
    effect: (effect) => {
      const frames = Object.keys(FRAME_TREE).filter((who) => who !== 'same')
      const { top, cross } = assertDecoysUntouched(
        effect,
        ['top', 'cross'],
        frames,
      )
      assert.deepEqual(top.iframes, [
        'login-iframe',
        'chain',
        'hidden-same',
        'hidden-cross',
      ])
      assert.deepEqual(cross, { ...pristine('cross'), gen: 2 })
    },
  },
  {
    name: 'goto on a child frame',
    source: `const same = page.frame('same')
    const url = same.url().replace('who=same', 'who=same&gen=5')
    return {
      goto: await settle(() => same.goto(url).then((response) => typeof response)),
      who: await settle(() => same.textContent('#who')),
    }`,
    perEngine: ['goto'],
    expected: { who: value('same') },
    effect: (effect) => {
      const { same } = assertDecoysUntouched(effect, ['same'])
      assert.deepEqual(same, { ...pristine('same'), gen: 5 })
    },
    neoEffect: (effect) => assertDecoysUntouched(effect, []),
    checkPlaywright: ({ result }) =>
      assert.deepEqual(result.value.goto, value('object'), 'a Response'),
    checkNeo: ({ result }) =>
      assert.deepEqual(result.value.goto, failed('error'), 'design §4.5'),
  },
]
test('the frame API lists and follows the same frames through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, cases)
})
