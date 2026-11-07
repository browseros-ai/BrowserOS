// locator.screenshot through Neo and through real Playwright on one BrowserOS neo, each case on
// two fresh tabs serving the same page (capture.mjs). Compared: the decoded image facts the
// script saw (type, width, height, alpha), the error's kind, the `path` file, and where the page
// ended up scrolled. Never pixels, checksums across engines, or error wording.
//
// Cases from Playwright's tests/page/elementhandle-screenshot.spec.ts at 26137face (the locator
// form, which resolves the element first), named by their ids.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { fileChecksum, withEngines } from './capture.mjs'
import { skip } from './harness.mjs'

const elements = `<!doctype html><title>elements</title><style>
  body { margin: 0 }
  #box { width: 120px; height: 80px; margin: 30px; background: red }
  #bordered { width: 50px; height: 40px; padding: 7px; border: 3px solid blue; margin: 10px; background: green }
  #huge { width: 300px; height: 900px; background: linear-gradient(red, blue) }
  .many { width: 10px; height: 10px; background: black }
  #zero { width: 50px; height: 0 }
  #late { width: 40px; height: 40px; background: orange; visibility: hidden }
  iframe { width: 200px; height: 150px; border: 0 }
</style>
<div id=box></div><div id=bordered></div><div class=many></div><div class=many></div>
<div id=zero></div><div id=late></div><div id=clear style="width:60px;height:30px"></div>
<iframe srcdoc="<style>body{margin:0}</style><div id=inner style='width:70px;height:20px;margin:5px;background:purple'></div>"></iframe>
<div id=huge></div>
<script>setTimeout(() => { document.getElementById('late').style.visibility = 'visible' }, 300)</script>`

const scrolled = `<!doctype html><title>scrolled</title>
<div style="height: 14px">oooo</div>
<style>
  div.above { border: 2px solid blue; background: red; height: 1500px }
  div.to-screenshot { border: 2px solid blue; background: green; width: 50px; height: 50px }
</style>
<div class="above"></div>
<div class="to-screenshot"></div>`

const cases = [
  {
    name: 'EH-074 should work',
    path: '/elements.html',
    source: "return describeImage(await page.locator('#box').screenshot())",
  },
  {
    name: 'EH-077 should take into account padding and border',
    path: '/elements.html',
    source:
      "return describeImage(await page.locator('#bordered').screenshot())",
  },
  {
    name: 'EH-079 should capture full element when larger than viewport',
    path: '/elements.html',
    source: "return describeImage(await page.locator('#huge').screenshot())",
  },
  {
    name: 'EH-080 should scroll element into view',
    path: '/scrolled.html',
    source:
      "return describeImage(await page.locator('div.to-screenshot').screenshot())",
    probe: 'window.scrollY > 0',
  },
  {
    name: 'EH-085 should wait for visible',
    path: '/elements.html',
    source: "return describeImage(await page.locator('#late').screenshot())",
  },
  {
    name: 'an element inside an iframe',
    path: '/elements.html',
    source:
      "return describeImage(await page.frameLocator('iframe').locator('#inner').screenshot())",
  },
  {
    name: 'EH-090/EH-091 path creates subdirectories; type wins over the extension',
    path: '/elements.html',
    source: `return {
      fromPath: describeImage(await page.locator('#box').screenshot({ path: at('el/box.jpg') })),
      typed: describeImage(await page.locator('#box').screenshot({ type: 'png', path: at('el/typed.jpg') })),
    }`,
    files: { 'el/box.jpg': 'fromPath', 'el/typed.jpg': 'typed' },
  },
  {
    name: 'omitBackground gives a transparent element PNG an alpha channel',
    path: '/elements.html',
    source: `return {
      omitted: describeImage(await page.locator('#clear').screenshot({ omitBackground: true })),
      opaque: describeImage(await page.locator('#clear').screenshot()),
    }`,
  },
  {
    name: 'a locator that matches two elements is a strict mode violation',
    path: '/elements.html',
    source: `try { await page.locator('.many').screenshot(); return 'taken' } catch (error) { return errorKind(error) }`,
    expect: { error: { name: 'Error', strict: true } },
  },
  {
    name: 'EH-084 should timeout waiting for visible',
    path: '/elements.html',
    source: `try { await page.locator('#zero').screenshot({ timeout: 1000 }); return 'taken' } catch (error) { return errorKind(error) }`,
    expect: { error: { name: 'TimeoutError', strict: false } },
  },
  {
    name: 'a locator that never matches times out',
    path: '/elements.html',
    source: `try { await page.locator('#missing').screenshot({ timeout: 500 }); return 'taken' } catch (error) { return errorKind(error) }`,
    expect: { error: { name: 'TimeoutError', strict: false } },
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

test('locator.screenshot reads the same through Neo and Playwright', {
  skip,
  timeout: 600_000,
}, async (t) => {
  const pages = { '/elements.html': elements, '/scrolled.html': scrolled }
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
