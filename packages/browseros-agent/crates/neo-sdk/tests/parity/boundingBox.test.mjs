// Locator.boundingBox waits for the element and answers its border box in the main frame's
// CSS pixels, as Playwright's `CRPage.getBoundingBox` does: a positioned div, an SVG rect, an
// element inside a bordered same-process frame, an element inside a cross-site frame with no
// border, and `null` for a display:none element. A strict locator, a missing element at its
// timeout and an invalid selector reject by kind.
//
// Inside a bordered cross-site (out-of-process) frame the two differ by design: Playwright adds
// the iframe's border-box origin to the frame's own coordinates, Neo its content-box origin
// (where the frame's viewport starts, as in a same-process frame), so Neo's box sits the
// iframe's border further right and down: frame-top.html's 1px (the frame case below), and
// neo-runtime's tests/element_ops_e2e.rs for a thicker border.
//
// From Playwright's specs at the pin (tests/page/elementhandle-bounding-box.spec.ts at
// 26137face): 'EH-001 should work', 'EH-002 should handle nested frames', 'EH-005 should return
// null for invisible elements', 'EH-006 should get bounding box of element inside a
// cross-origin iframe' and 'EH-008 should work with SVG nodes'.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareFrameCases } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

test('boundingBox answers the border box of the element it names as Playwright does', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/bounding-box.html', import.meta.url),
      'utf8',
    ),
    source: `
      const round = (b) => b && Object.fromEntries(Object.entries(b).map(([k, v]) => [k, Math.round(v * 100) / 100]))
      const boxes = {
        box: await page.locator('#box').boundingBox(),
        hidden: await page.locator('#hidden').boundingBox({ timeout: 5000 }),
        rect: round(await page.locator('#rect').boundingBox(null)),
        inner: round(await page.frameLocator('#same').locator('#inner').boundingBox()),
        cross: round(await page.frameLocator('#cross').locator('h1').boundingBox()),
      }
      const failures = {}
      for (const [name, act] of [
        ['strict', () => page.locator('.dup').boundingBox()],
        ['missing', () => page.locator('#missing').boundingBox({ timeout: 500 })],
        ['invalid', () => page.locator('[').boundingBox()],
      ]) {
        try { await act(); failures[name] = 'resolved' } catch (error) { failures[name] = errorCategory(error) }
      }
      return { boxes, failures }
    `,
    probe: 'window.__facts()',
  })
  const { boxes, failures } = outcome.value
  assert.deepEqual(boxes.box, { x: 100, y: 50, width: 50, height: 50 })
  assert.equal(boxes.hidden, null)
  assert.deepEqual(boxes.rect, { x: 30, y: 170, width: 200, height: 100 })
  // The same-process frame's content box starts at 300 + 2 (border) + 3 (padding).
  assert.deepEqual(boxes.inner, { x: 315, y: 35, width: 180, height: 18 })
  assert.equal(
    boxes.cross.x,
    20 + 8,
    'the cross frame starts at 20, its h1 at 8',
  )
  assert.deepEqual(failures, {
    strict: 'Error:strict',
    missing: 'TimeoutError',
    invalid: 'Error',
  })
})

/** boundingBox inside frame-top.html's frames (frames.mjs), whose iframes have a 1px border:
 * a same-process frame answers the same box through both engines; in an out-of-process frame
 * Neo's box is one border further right and down per out-of-process hop (the Deviation
 * above). */
const frameCases = [
  {
    name: 'boundingBox answers the box of the element in the frame it names',
    source: `const box = (locator) => settle(async () => {
      const b = await locator.boundingBox()
      return b && Object.fromEntries(Object.entries(b).map(([k, v]) => [k, Math.round(v * 100) / 100]))
    })
    return {
      same: await box(page.frameLocator('#same').locator('#send')),
      cross: await box(page.frameLocator('#login-iframe').locator('#send')),
      grandchild: await box(page.frame('grandchild').locator('#send')),
      missingFrame: await settle(() => page.frameLocator('#nope').locator('#send').boundingBox({ timeout: 500 })),
    }`,
    expected: {
      same: (box) => assert.ok(box.value.width > 0 && box.value.height > 0),
      missingFrame: { error: 'timeout', name: 'TimeoutError' },
    },
    perEngine: ['cross', 'grandchild'],
    checkPlaywright: ({ result }) => {
      for (const who of ['cross', 'grandchild'])
        assert.ok(result.value[who].value.width > 0, `${who} has a box`)
    },
    checkNeo: (neo, playwright) => {
      // One out-of-process hop for cross, two for grandchild (127.0.0.1 inside localhost),
      // each behind an iframe with a 1px border; the size is the same.
      for (const [who, borders] of [
        ['cross', 1],
        ['grandchild', 2],
      ]) {
        const [n, p] = [
          neo.result.value[who].value,
          playwright.result.value[who].value,
        ]
        assert.deepEqual(
          {
            dx: Math.round((n.x - p.x) * 100) / 100,
            dy: Math.round((n.y - p.y) * 100) / 100,
            width: n.width,
            height: n.height,
          },
          { dx: borders, dy: borders, width: p.width, height: p.height },
          `${who}: Neo's box sits the iframe borders further right and down`,
        )
      }
    },
  },
]

test('boundingBox inside frames answers through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
