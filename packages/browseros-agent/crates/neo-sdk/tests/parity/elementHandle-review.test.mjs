// Neo: independent E1 correctness cases. Compare public values, frame identity and error
// categories with the pinned Playwright; no error wording or timing assertions.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { withFrames } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

const observe = `
const observe = async (handle) => {
  const result = {}
  for (const method of ['textContent', 'innerText', 'innerHTML', 'getAttribute', 'inputValue',
    'isVisible', 'isHidden', 'isEnabled', 'isDisabled', 'isEditable', 'isChecked',
    'boundingBox', 'ownerFrame', 'contentFrame']) {
    try {
      let value = await handle[method](...(method === 'getAttribute' ? ['data-id'] : []))
      if (method === 'ownerFrame' || method === 'contentFrame')
        value = value === null ? null : value === page.mainFrame() ? 'main' : value.name()
      result[method] = { value }
    } catch (error) { result[method] = { error: error.name } }
  }
  return result
}
`
const html = `<!doctype html><title>E1 review</title>
<input id="target" data-id="original" value="original value" type="checkbox" checked>
<div id="text">original <b>text</b></div>`
const compare = (t, source) =>
  compareMethod(t, {
    html,
    source: observe + source,
    probe:
      '[...document.querySelectorAll("#target, #text")].map(element => element.outerHTML)',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify({ playwright, neo }))
      assert.deepEqual(neo, playwright)
    },
  })

test('a replaced selector target leaves the original handle unchanged', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compare(
    t,
    `
    const handle = await page.$('#target')
    const text = await page.$('#text')
    await handle.evaluate(element => {
      const replacement = element.cloneNode(true)
      replacement.value = 'replacement value'
      replacement.setAttribute('data-id', 'replacement')
      element.replaceWith(replacement)
    })
    await text.evaluate(element => { element.outerHTML = '<div id="text">replacement text</div>' })
    return { input: await observe(handle), text: {
      textContent: await text.textContent(), innerText: await text.innerText(),
      innerHTML: await text.innerHTML(),
    }, liveInput: await page.inputValue('#target') }
  `,
  )
})

test('methods on a disposed handle match Playwright error categories', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compare(
    t,
    `
    const handle = await page.$('#target')
    await handle.dispose()
    return await observe(handle)
  `,
  )
})

test('methods on a handle from the preceding document match Playwright', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compare(
    t,
    `
    const handle = await page.$('#target')
    await page.goto(page.url() + '?next=1')
    return await observe(handle)
  `,
  )
})

test('ownerFrame finds a document on another page without a prior call on that page', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compare(
    t,
    `
    const handle = await page.$('#text')
    await page.evaluate(url => { window.__child = window.open(url, 'e1-owner') }, page.url() + '?owner=2')
    await page.waitForFunction(() => window.__child && window.__child.document.readyState === 'complete' &&
      window.__child.location.search === '?owner=2')
    const other = page.context().pages().at(-1)
    try {
      await handle.evaluate(element => {
        const doc = window.__child.document
        doc.body.append(doc.adoptNode(element))
      })
      const owner = await handle.ownerFrame()
      const before = { found: owner !== null, correct: owner === other.mainFrame() }
      await other.title()
      const after = await handle.ownerFrame()
      return { before, after: { found: after !== null, correct: after === other.mainFrame() } }
    } finally { await other.close() }
  `,
  )
})

test('frame adoption and OOPIF boxes match Playwright', {
  skip,
  timeout: 600_000,
}, async (t) => {
  await withFrames(t, {}, async (run) => {
    const cases = [
      {
        name: 'ownerFrame follows an element adopted into another document',
        source: `
        const handle = await page.$('#not-a-frame')
        const same = page.frames().find(frame => frame.name() === 'same')
        await handle.evaluate(element => {
          const child = document.querySelector('#same').contentDocument
          child.body.append(child.adoptNode(element))
        })
        return { owner: (await handle.ownerFrame()) === same,
          text: await handle.textContent(), content: await handle.contentFrame() }
      `,
      },
      {
        name: 'bordered OOPIF and hidden OOPIF boxes use the same geometry',
        source: `
        await page.evaluate(() => {
          document.querySelector('#login-iframe').style.cssText +=
            '; border: 11px solid black; padding: 7px;'
        })
        const cross = page.frames().find(frame => frame.name() === 'cross')
        const hidden = page.frames().find(frame => frame.name() === 'hidden-cross')
        const handle = await cross.$('#email')
        const hiddenHandle = await hidden.$('#email')
        return { owner: (await handle.ownerFrame()) === cross,
          box: await handle.boundingBox(), locator: await cross.locator('#email').boundingBox(),
          hidden: await hiddenHandle.boundingBox() }
      `,
      },
    ]
    for (const c of cases)
      await t.test(c.name, async () => {
        const { playwright, neo } = await run(c)
        t.diagnostic(
          JSON.stringify({ playwright: playwright.result, neo: neo.result }),
        )
        assert.ok(
          'value' in playwright.result,
          JSON.stringify(playwright.result),
        )
        assert.deepEqual(neo.result, playwright.result)
        assert.deepEqual(neo.effect, playwright.effect)
      })
  })
})
