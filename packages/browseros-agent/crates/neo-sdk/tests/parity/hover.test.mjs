// Hover through Neo and through Playwright on fresh tests/pages/pointer-actions.html pages. Every
// receiver returns undefined and leaves the pointer over the element it named, and the page saw
// the same trusted mouse events on the same elements; a hidden and a covered element time out
// without being hovered, and a strict locator over two matches fails at once. A disabled button
// is hovered: hover waits for visible and stable, never enabled (the regression a click-shaped
// policy would bring). Positions and coordinates are not compared.
//
// From Playwright's specs at the pin: tests/page/locator-misc-1.spec.ts 'LO-127 should hover
// @smoke', tests/page/page-mouse.spec.ts 'MO-008 should trigger hover state' and 'MO-009 should
// trigger hover state on disabled button'.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareFrameCases } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

test('hover returns undefined and moves the pointer onto the element', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/pointer-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      const voids = [
        await page.locator('#other').hover() === undefined,
        await page.hover('#target', null) === undefined,
        await page.mainFrame().hover('.dup') === undefined,
        await page.locator('#disabled').hover({ timeout: 5000 }) === undefined,
      ]
      const failures = {}
      for (const [name, act] of [
        ['strict', () => page.locator('.dup').hover()],
        ['hidden', () => page.locator('#hidden').hover({ timeout: 500 })],
        ['covered', () => page.locator('#covered').hover({ timeout: 1000 })],
      ]) {
        try { await act(); failures[name] = 'resolved' } catch (error) { failures[name] = errorCategory(error) }
      }
      voids.push(await page.locator('#target').hover({ position: { x: 4, y: 4 } }) === undefined)
      return { voids, failures }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value, {
    voids: [true, true, true, true, true],
    failures: {
      strict: 'Error:strict',
      hidden: 'TimeoutError',
      covered: 'TimeoutError',
    },
  })
  assert.deepEqual(
    outcome.calls.map((call) => call.delivery),
    ['done', 'done', 'done', 'done', 'notSent', 'notSent', 'notSent', 'done'],
    "Neo's delivery records",
  )
})

/**
 * The regression for the interceptor's action kind: the overlay appears at the hover's own
 * pointermove, so the interceptor finds it when it judges the mousemove, blocks that move and
 * the hover moves again. An interceptor armed for a click's events judges no mousemove, and
 * would report the blocked move as the hover.
 */
test('a hover whose move an overlay takes is moved again until it reaches the element', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/hover-blocked-once.html', import.meta.url),
      'utf8',
    ),
    source: `return await page.locator('#target').hover() === undefined`,
    probe:
      '({ moved: window.__moves.length > 1 ? "again" : "once", last: window.__moves.at(-1) })',
    effects: ({ playwright, neo }) => {
      assert.deepEqual(
        playwright,
        { moved: 'again', last: 'target' },
        "Playwright's hover moved again after its interceptor blocked the first move",
      )
      assert.deepEqual(neo, playwright, 'browser effect')
    },
  })
  assert.equal(outcome.value, true)
  assert.deepEqual(
    outcome.calls.map((call) => call.delivery),
    ['done'],
  )
})

/** Every frame's hovered elements with an id. */
const HOVERED =
  '[...document.querySelectorAll(":hover")].map((element) => element.id).filter(Boolean)'

/**
 * Hover inside frames, on tabs of frame-top.html (frames.mjs): the pointer moves over the Send
 * button of the same-origin frame, then of the cross-site out-of-process frame's 127.0.0.1
 * child, which ends hovered. Chromium's hover state across processes (the first frame keeps
 * its `:hover`) is whatever it is: every frame must read the same in both engines.
 */
const frameCases = [
  {
    name: 'a hover inside an out-of-process frame hovers that element',
    source: `const login = page.frameLocator('#login-iframe')
    return {
      same: await settle(() => page.frameLocator('#same').locator('#send').hover()),
      grandchild: await settle(() => login.frameLocator('#grandchild').locator('#send').hover()),
    }`,
    probe: HOVERED,
    expected: { same: 'undefined', grandchild: 'undefined' },
    effect: (effect) => {
      assert.deepEqual(effect.grandchild, ['send'], 'the hovered button')
    },
    deliveries: { hover: ['done', 'done'] },
  },
]

test('hover inside frames moves onto the same element through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})

// Review regression: scroll:none must also constrain the containing OOPIF.
test('review: hover with scroll none leaves an offscreen frame offscreen', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html: `<!doctype html><style>body { margin: 0 }</style><script>
      if (location.hostname === '127.0.0.1') {
        document.write('<div style="height:2000px"></div><iframe id="frame" src="http://localhost:' + location.port + '/method.html"></iframe>')
      } else {
        document.write('<button id="target">Target</button>')
      }
    </script>`,
    source: `
      try {
        await page.frameLocator('#frame').locator('#target').hover({ scroll: 'none', timeout: 1500 })
        return 'hovered'
      } catch (error) {
        return errorCategory(error)
      }
    `,
    probe: '({ scrolled: scrollY > 0 })',
  })
})

test('review: hover rejects an invalid position before sending input', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: '<button id="target" onmousemove="this.dataset.moved = 1">Target</button>',
    source: `
      try {
        await page.locator('#target').hover({ position: { x: 'bad', y: 0 } })
        return 'hovered'
      } catch (error) {
        return errorCategory(error)
      }
    `,
    probe: 'document.querySelector("#target").dataset.moved ?? null',
  })
  assert.equal(outcome.value, 'Error')
  assert.equal(outcome.calls[0].delivery, 'notSent')
})
