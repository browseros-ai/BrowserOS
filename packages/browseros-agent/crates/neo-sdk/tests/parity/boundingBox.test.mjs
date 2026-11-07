// Locator.boundingBox waits for the element and answers its border box in the main frame's
// CSS pixels, as Playwright's `CRPage.getBoundingBox` does: a positioned div, an SVG rect, an
// element inside a bordered same-process frame, an element inside a bordered and padded
// cross-site frame, and `null` for a display:none element. A strict locator, a missing element
// at its timeout and an invalid selector reject by kind.
//
// Inside a cross-site (out-of-process) frame Playwright adds the iframe's border-box origin to
// the frame's own coordinates (`_framePosition`), not where the frame's viewport starts; Neo
// places boxes the same way (input still aims by the content box), so every case below
// compares the two engines' boxes directly, nested frames of both kinds included.
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
    "the cross iframe's border box starts at 20, its h1 at 8: Playwright skips the border and padding",
  )
  assert.deepEqual(failures, {
    strict: 'Error:strict',
    missing: 'TimeoutError',
    invalid: 'Error',
  })
})

/** boundingBox inside frame-top.html's frames (frames.mjs), whose iframes have a 1px border, and
 * inside frames a case adds with a thicker border and padding: every box is Playwright's, one
 * and two out-of-process hops, an out-of-process frame inside a same-process one, and a
 * same-process frame inside an out-of-process one. */
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
      // One out-of-process hop for cross, two for grandchild (127.0.0.1 inside localhost).
      cross: (box) => assert.ok(box.value.width > 0 && box.value.height > 0),
      grandchild: (box) =>
        assert.ok(box.value.width > 0 && box.value.height > 0),
      missingFrame: { error: 'timeout', name: 'TimeoutError' },
    },
  },
  {
    name: 'boundingBox in mixed nested frames places boxes as Playwright does',
    source: `const port = page.url().match(/:(\\d+)\\//)[1]
    // An iframe with a 6px border and 4px padding, added in the frame \`body\` is in.
    const add = (body, name, host) => body.evaluate((element, [frameName, src]) => new Promise((resolve) => {
      const frame = document.createElement('iframe')
      frame.name = frameName
      frame.style.cssText = 'border: 6px solid #888; padding: 4px'
      frame.onload = () => resolve(true)
      frame.src = src
      element.append(frame)
    }), [name, 'http://' + host + ':' + port + '/frame-child.html?who=' + name])
    await add(page.frameLocator('#same').locator('body'), 'same-cross', 'localhost')
    await add(page.frameLocator('#login-iframe').locator('body'), 'cross-same', 'localhost')
    const box = (locator) => settle(async () => {
      const b = await locator.boundingBox()
      return b && Object.fromEntries(Object.entries(b).map(([k, v]) => [k, Math.round(v * 100) / 100]))
    })
    return {
      sameCross: await box(page.frameLocator('#same').frameLocator('iframe[name="same-cross"]').locator('#send')),
      crossSame: await box(page.frameLocator('#login-iframe').frameLocator('iframe[name="cross-same"]').locator('#send')),
    }`,
    expected: {
      // An out-of-process frame inside a same-process one; a same-process frame inside an
      // out-of-process one.
      sameCross: (box) =>
        assert.ok(box.value.width > 0 && box.value.height > 0),
      crossSame: (box) =>
        assert.ok(box.value.width > 0 && box.value.height > 0),
    },
  },
]

test('boundingBox inside frames answers through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
