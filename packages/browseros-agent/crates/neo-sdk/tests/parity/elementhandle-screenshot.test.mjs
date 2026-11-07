// elementHandle.screenshot through Neo and through real Playwright on one BrowserOS neo, each
// case on two fresh tabs serving the same page (capture.mjs). Compared: the decoded image facts
// the script saw (type, width, height, alpha), the error's category, the `path` files, and the
// page's state afterwards. Never pixels, checksums across engines, or error wording.
//
// Cases from Playwright's tests/page/elementhandle-screenshot.spec.ts at 26137face, named by
// their ids; `setContent` and `setViewportSize` become fixture pages on the browser's own
// viewport, and `exposeFunction` a page flag.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { fileChecksum, withEngines } from './capture.mjs'
import { skip } from './harness.mjs'

const grid = `<!doctype html><title>grid</title><style>
  body { margin: 0; padding: 0; width: 3000px; height: 3000px }
  .box { display: inline-flex; width: 50px; height: 50px; box-sizing: border-box; border: 1px solid darkgray; background: teal }
  .box.animation { animation: slide 300ms linear }
  @keyframes slide { from { transform: translateX(80px) } to { transform: none } }
  ::-webkit-scrollbar { display: none }
</style>
<script>
document.addEventListener('DOMContentLoaded', () => {
  for (let i = 0; i < 40; ++i) {
    const box = document.createElement('div')
    box.className = 'box'
    box.textContent = String(i)
    document.body.append(box)
  }
})
</script>`

const pages = {
  '/grid.html': grid,
  '/padded.html': `<!doctype html><div style="height: 14px">oooo</div>
<style>div { border: 2px solid blue; background: green; width: 50px; height: 50px }</style>
<div id="d"></div>`,
  '/large.html': `<!doctype html><div style="height: 14px">oooo</div>
<style>
  div.to-screenshot { border: 1px solid blue; width: 600px; height: 600px; margin-left: 50px }
  ::-webkit-scrollbar { display: none }
</style>
<div class="to-screenshot"></div><div class="to-screenshot"></div><div class="to-screenshot"></div>`,
  '/scrolled.html': `<!doctype html><div style="height: 14px">oooo</div>
<style>
  div.above { border: 2px solid blue; background: red; height: 1500px }
  div.to-screenshot { border: 2px solid blue; background: green; width: 50px; height: 50px }
</style>
<div class="above"></div><div class="to-screenshot"></div>`,
  '/far.html': `<!doctype html><div style="height: 14px">oooo</div>
<style>
  div.above { border: 2px solid blue; background: red; height: 15000px }
  div.to-screenshot { border: 2px solid blue; background: green; width: 50px; height: 50px }
</style>
<div class="above"></div><div class="to-screenshot"></div>`,
  '/rotated.html': `<!doctype html><div style="position:absolute; top: 100px; left: 100px; width: 100px; height: 100px; background: green; transform: rotateZ(200deg);">&nbsp;</div>`,
  '/detached.html': '<!doctype html><h1>remove this</h1>',
  '/zero.html': '<!doctype html><div style="width: 50px; height: 0"></div>',
  '/fractional.html':
    '<!doctype html><div style="width:48.51px;height:19.8px;border:1px solid black;"></div>',
  '/offset.html':
    '<!doctype html><div style="position:absolute; top: 10.3px; left: 20.4px;width:50.3px;height:20.2px;border:1px solid black;"></div>',
  '/button.html': '<!doctype html><button disabled>Click me</button>',
  '/framed.html': `<!doctype html><style>body { margin: 0 } iframe { width: 200px; height: 150px; border: 0; margin: 20px }</style>
<div id=box style="width:60px;height:30px;background:red"></div>
<div id=clear style="width:60px;height:30px"></div>
<iframe srcdoc="<style>body{margin:0}</style><div id=inner style='width:70px;height:20px;margin:5px;background:purple'></div>"></iframe>`,
}

const third = "const handle = await page.$('.box:nth-of-type(3)')"
const rafraf = `const rafraf = () => page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))`

const cases = [
  {
    name: 'EH-074 should work',
    path: '/grid.html',
    source: `await page.evaluate(() => window.scrollBy(50, 100))
${third}
return describeImage(await handle.screenshot())`,
    probe: '[window.scrollX, window.scrollY].join()',
  },
  {
    name: 'EH-075 should work with webp',
    path: '/grid.html',
    source: `${third}
return {
  webp: describeImage(await handle.screenshot({ type: 'webp' })),
  png: describeImage(await handle.screenshot({ type: 'png' })),
}`,
  },
  {
    name: 'EH-076 should work when main world busts JSON.stringify',
    path: '/grid.html',
    source: `await page.evaluate(() => {
  window.scrollBy(50, 100)
  JSON.stringify = () => undefined
})
${third}
return describeImage(await handle.screenshot())`,
  },
  {
    name: 'EH-077 should take into account padding and border',
    path: '/padded.html',
    source: "return describeImage(await (await page.$('div#d')).screenshot())",
  },
  {
    name: 'EH-078 should capture full element when larger than viewport in parallel',
    path: '/large.html',
    source: `const handles = await page.$$('div.to-screenshot')
const shots = await Promise.all(handles.map((handle) => handle.screenshot()))
return shots.map(describeImage)`,
    probe: '[window.innerWidth, window.innerHeight].join()',
  },
  {
    name: 'EH-079 should capture full element when larger than viewport',
    path: '/large.html',
    source:
      "return describeImage(await (await page.$('div.to-screenshot')).screenshot())",
    probe: '[window.innerWidth, window.innerHeight].join()',
  },
  {
    name: 'EH-080 should scroll element into view',
    path: '/scrolled.html',
    source:
      "return describeImage(await (await page.$('div.to-screenshot')).screenshot())",
    probe: 'window.scrollY > 0',
  },
  {
    name: 'EH-081 should scroll 15000px into view',
    path: '/far.html',
    source:
      "return describeImage(await (await page.$('div.to-screenshot')).screenshot())",
    probe: 'window.scrollY > 14000',
  },
  {
    name: 'EH-082 should work with a rotated element',
    path: '/rotated.html',
    source: "return describeImage(await (await page.$('div')).screenshot())",
  },
  {
    name: 'EH-083 should fail to screenshot a detached element',
    path: '/detached.html',
    source: `const handle = await page.$('h1')
await page.evaluate((element) => element.remove(), handle)
try {
  await handle.screenshot()
  return 'taken'
} catch (error) {
  return { name: error.name, detached: /Element is not attached to the DOM/.test(error.message) }
}`,
    expect: { name: 'Error', detached: true },
  },
  {
    name: 'EH-084 should timeout waiting for visible',
    path: '/zero.html',
    source: `const handle = await page.$('div')
try {
  await handle.screenshot({ timeout: 1000 })
  return 'taken'
} catch (error) {
  return {
    name: error.name,
    timeout: /elementHandle\\.screenshot: Timeout 1000ms exceeded/.test(error.message),
    notVisible: /element is not visible/.test(error.message),
  }
}`,
    expect: { name: 'TimeoutError', timeout: true, notVisible: true },
  },
  {
    name: 'EH-085 should wait for visible',
    path: '/grid.html',
    source: `await page.evaluate(() => window.scrollBy(50, 100))
${third}
${rafraf}
await handle.evaluate((element) => { element.style.visibility = 'hidden' })
let done = false
const pending = handle.screenshot().then((bytes) => {
  done = true
  return bytes
})
for (let i = 0; i < 10; ++i) await rafraf()
const waited = !done
await handle.evaluate((element) => { element.style.visibility = 'visible' })
return { waited, image: describeImage(await pending) }`,
  },
  {
    name: 'EH-086 should work for an element with fractional dimensions',
    path: '/fractional.html',
    source: "return describeImage(await (await page.$('div')).screenshot())",
  },
  {
    name: 'EH-087 should work for an element with an offset',
    path: '/offset.html',
    source: "return describeImage(await (await page.$('div')).screenshot())",
  },
  {
    name: 'EH-088 should wait for element to stop moving',
    path: '/grid.html',
    source: `${third}
${rafraf}
await handle.evaluate((element) => element.classList.add('animation'))
await rafraf()
return describeImage(await handle.screenshot())`,
  },
  {
    name: 'EH-089 should take screenshot of disabled button',
    path: '/button.html',
    source: "return describeImage(await (await page.$('button')).screenshot())",
  },
  {
    name: 'EH-090/EH-091 path creates subdirectories; type wins over the extension',
    path: '/grid.html',
    source: `${third}
return {
  fromPath: describeImage(await handle.screenshot({ path: at('these/are/directories/screenshot.png') })),
  typed: describeImage(await handle.screenshot({ path: at('file.png'), type: 'jpeg' })),
}`,
    files: {
      'these/are/directories/screenshot.png': 'fromPath',
      'file.png': 'typed',
    },
  },
  {
    name: 'EH-092 should not issue resize event',
    path: '/grid.html',
    source: `await page.evaluate(() => {
  window.__resized = false
  window.addEventListener('resize', () => { window.__resized = true })
})
${third}
await handle.screenshot()
return await page.evaluate(() => window.__resized)`,
  },
  {
    name: 'an element handle inside an iframe',
    path: '/framed.html',
    source: `const frame = page.frames().find((frame) => frame !== page.mainFrame())
return describeImage(await (await frame.$('#inner')).screenshot())`,
  },
  {
    name: 'omitBackground, mask and style on an element handle',
    path: '/framed.html',
    source: `const box = await page.$('#box')
const clear = await page.$('#clear')
return {
  omitted: describeImage(await clear.screenshot({ omitBackground: true })),
  opaque: describeImage(await clear.screenshot()),
  masked: describeImage(await box.screenshot({ mask: [page.locator('#box')], maskColor: '#00FF00' })),
  styled: describeImage(await box.screenshot({ style: '#box { width: 90px !important }' })),
}`,
    probe: "getComputedStyle(document.getElementById('box')).width",
  },
  {
    name: 'a handle whose document went away is not attached',
    path: '/detached.html',
    source: `const handle = await page.$('h1')
await page.goto(page.url())
try {
  await handle.screenshot()
  return 'taken'
} catch (error) {
  return { name: error.name, detached: /Element is not attached to the DOM/.test(error.message) }
}`,
  },
  {
    name: 'a disposed handle cannot be captured',
    path: '/detached.html',
    source: `const handle = await page.$('h1')
await handle.dispose()
try {
  await handle.screenshot()
  return 'taken'
} catch (error) {
  return { name: error.name, closed: /closed/i.test(error.message), disposed: /disposed/i.test(error.message) }
}`,
  },
  {
    name: 'a JSHandle has no screenshot',
    path: '/framed.html',
    source: `const object = await page.evaluateHandle(() => ({ answer: 42 }))
return { asElement: object.asElement(), screenshot: typeof object.screenshot }`,
    expect: { asElement: null, screenshot: 'undefined' },
  },
]

/** `value` without checksums: bytes are compared within an engine, never across them. */
function facts(value) {
  if (Array.isArray(value)) return value.map(facts)
  if (value && typeof value === 'object')
    return Object.fromEntries(
      Object.entries(value)
        .filter(([key]) => key !== 'checksum')
        .map(([key, item]) => [key, facts(item)]),
    )
  return value
}

function seen(engine) {
  return engine.error ? { error: engine.error } : { value: facts(engine.value) }
}

test('elementHandle.screenshot reads the same through Neo and Playwright', {
  skip,
  timeout: 900_000,
}, async (t) => {
  await withEngines(t, { pages }, async (run) => {
    for (const c of cases) {
      await t.test(c.name, async () => {
        const { playwright, neo } = await run(c.path, c.source, c.probe)
        t.diagnostic(
          `${c.name}: ${JSON.stringify(seen(neo))} probe ${JSON.stringify(neo.probe)}`,
        )
        assert.deepEqual(seen(neo), seen(playwright), 'what the script saw')
        if (c.expect)
          assert.deepEqual(neo.value, c.expect, 'the case is the one it names')
        for (const [file, key] of Object.entries(c.files ?? {})) {
          for (const [side, engine] of [
            ['Playwright', playwright],
            ['Neo', neo],
          ]) {
            const written = engine.file(file)
            assert.ok(written, `${side} wrote ${file}`)
            assert.equal(
              fileChecksum(written),
              engine.value[key].checksum,
              `${side}'s ${file} holds the returned bytes`,
            )
          }
        }
        assert.deepEqual(neo.probe, playwright.probe, 'where the page ended up')
      })
    }
  })
})
