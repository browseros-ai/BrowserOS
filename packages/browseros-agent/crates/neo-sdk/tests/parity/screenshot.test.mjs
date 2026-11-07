// page.screenshot through Neo and through real Playwright on one BrowserOS neo, each case on two
// fresh tabs serving the same page (capture.mjs). Compared: what the script saw of its own bytes
// (image type, width and height, whether a PNG has an alpha channel), the error's kind, the
// file a `path` wrote (present and equal to the returned bytes, or absent), and the page's state
// after the screenshot. Never pixels, checksums across engines, or error wording.
//
// Cases from Playwright's tests/page/page-screenshot.spec.ts and tests/library/screenshot.spec.ts
// at 26137face, named by their ids.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { fileChecksum, withEngines } from './capture.mjs'
import { skip } from './harness.mjs'

const grid = `<!doctype html><title>grid</title><style>
  body { margin: 0 }
  .box { display: inline-block; width: 50px; height: 50px; margin: 0; background: hsl(200, 70%, 50%) }
  #tall { width: 700px; height: 1500px; background: linear-gradient(#f80, #08f) }
</style><div id=tall>${'<div class=box></div>'.repeat(20)}</div>`

const transparent = `<!doctype html><title>transparent</title><style>
  body { margin: 0 } div { width: 300px; height: 100px }
</style><div style="background:black"></div><div style="background:white"></div><div style="background:transparent"></div>`

const prepared = `<!doctype html><title>prepared</title><style>
  @keyframes spin { from { transform: rotate(0) } to { transform: rotate(360deg) } }
  #spinner { width: 40px; height: 40px; background: red; animation: spin 2s linear infinite }
  #hidden-by-style { width: 80px; height: 30px; background: blue }
</style><div id=spinner></div><input id=field value="caret" style="caret-color: rgb(255, 0, 0)">
<div id=hidden-by-style></div><div id=box style="width:60px;height:60px;background:green"></div>
<script>
  document.getElementById('field').focus()
  // What the screenshot did to the document, as the page's own listeners see it: the nodes and
  // inline styles it changed, in order (BrowserOS's own takeover overlay is left out), and the
  // animation events, which arrive on the page's own schedule and are only counted.
  window.__mutations = []
  window.__cancelled = 0
  const watched = (node) => node.nodeType === 1 && ['STYLE', 'X-PW-GLASS'].includes(node.nodeName)
  new MutationObserver((records) => {
    for (const record of records) {
      for (const node of record.addedNodes)
        if (watched(node)) window.__mutations.push('+' + node.nodeName.toLowerCase())
      for (const node of record.removedNodes)
        if (watched(node)) window.__mutations.push('-' + node.nodeName.toLowerCase())
      if (record.type === 'attributes' && record.target.id === 'field')
        window.__mutations.push('caret ' + record.target.style.getPropertyValue('caret-color'))
    }
  }).observe(document.documentElement, {
    childList: true, subtree: true, attributes: true, attributeFilter: ['style'],
  })
  document.getElementById('spinner').addEventListener('animationcancel', () => window.__cancelled++)
</script>`

// A frame whose document records the mask overlay Playwright's injected script adds to it.
const framed = `<!doctype html><title>framed</title><script>window.__frameMutations = []</script>
<iframe srcdoc="<style>body{margin:0}</style><div id=inner style='width:80px;height:40px;background:purple'></div><script>new MutationObserver((records) => { for (const record of records) { for (const node of record.addedNodes) if (node.nodeName === 'X-PW-GLASS') parent.__frameMutations.push('+glass'); for (const node of record.removedNodes) if (node.nodeName === 'X-PW-GLASS') parent.__frameMutations.push('-glass') } }).observe(document.documentElement, { childList: true })</script>"></iframe>`

// The body removes itself while it parses, so the document has none once loaded. The head
// counts the <style> elements a screenshot's \`style\` option adds.
const bodiless = `<!doctype html><title>no body</title><script>
  window.__stylesAdded = 0
  new MutationObserver((records) => {
    for (const record of records)
      for (const node of record.addedNodes) if (node.nodeName === 'STYLE') window.__stylesAdded++
  }).observe(document.documentElement, { childList: true })
</script><body><p>gone</p><script>document.body.remove()</script></body>`

/** What a page looks like after a screenshot prepared it: Playwright and Neo restore it all. */
const RESTORED = `(() => ({
  styles: document.querySelectorAll('style').length,
  caret: getComputedStyle(document.getElementById('field')).caretColor,
  inlineCaret: document.getElementById('field').style.getPropertyValue('caret-color'),
  animations: document.getAnimations().map((a) => a.playState),
  focused: document.activeElement && document.activeElement.id,
  masks: document.querySelectorAll('x-pw-glass').length,
  hiddenByStyle: getComputedStyle(document.getElementById('hidden-by-style')).visibility,
  mutations: window.__mutations,
  cancelled: window.__cancelled,
}))()`

/**
 * Each case: the page, the script (it returns decoded facts or an error kind), the file a
 * `path` should leave (`file`), or must not (`noFile`), and a page probe.
 */
const cases = [
  {
    name: 'SS-001 should work: the viewport as PNG',
    path: '/grid.html',
    source: 'return describeImage(await page.screenshot())',
  },
  {
    name: 'SS-005 should clip rect',
    path: '/grid.html',
    source:
      'return describeImage(await page.screenshot({ clip: { x: 50, y: 100, width: 150, height: 100 } }))',
  },
  {
    name: 'SS-011/SS-013 fullPage screenshots leave the viewport as it was',
    path: '/grid.html',
    source: 'return describeImage(await page.screenshot({ fullPage: true }))',
    probe: '`${innerWidth}x${innerHeight}`',
  },
  {
    name: 'SS-006 should clip rect with fullPage',
    path: '/grid.html',
    source:
      'return describeImage(await page.screenshot({ fullPage: true, clip: { x: 50, y: 1200, width: 150, height: 100 } }))',
  },
  {
    name: 'SS-031/SS-035 path detects jpeg, quality applies, the file holds the bytes',
    path: '/grid.html',
    source:
      "return describeImage(await page.screenshot({ path: at('shot.jpeg'), quality: 50 }))",
    file: 'shot.jpeg',
  },
  {
    name: 'SS-035 quality changes the JPEG each engine writes',
    path: '/grid.html',
    source: `const low = await page.screenshot({ type: 'jpeg', quality: 5 })
    const high = await page.screenshot({ type: 'jpeg', quality: 95 })
    return { low: describeImage(low), high: describeImage(high), smaller: low.length < high.length }`,
  },
  {
    name: 'SS-036 should prefer type over extension',
    path: '/grid.html',
    source:
      "return describeImage(await page.screenshot({ path: at('shot.png'), type: 'jpeg' }))",
    file: 'shot.png',
  },
  {
    name: 'SS-030 path option should create subdirectories',
    path: '/grid.html',
    source:
      "return describeImage(await page.screenshot({ path: at('nested/dir/shot.png') }))",
    file: 'nested/dir/shot.png',
  },
  {
    name: 'SS-016 should produce a valid webp screenshot',
    path: '/grid.html',
    source: "return describeImage(await page.screenshot({ type: 'webp' }))",
  },
  {
    name: 'SS-014/SS-015 omitBackground: alpha in PNG, none in JPEG',
    path: '/transparent.html',
    source: `return {
      opaque: describeImage(await page.screenshot()),
      omitted: describeImage(await page.screenshot({ omitBackground: true })),
      jpeg: describeImage(await page.screenshot({ omitBackground: true, type: 'jpeg' })),
      after: describeImage(await page.screenshot()),
    }`,
  },
  {
    name: 'prepared options and masks leave the page as it was',
    path: '/prepared.html',
    source: `const shot = await page.screenshot({
      animations: 'disabled', caret: 'hide', style: '#hidden-by-style { visibility: hidden }',
      mask: [page.locator('#box'), page.locator('#spinner')], maskColor: '#00FF00',
    })
    const initial = await page.screenshot({ caret: 'initial' })
    return { shot: describeImage(shot), initial: describeImage(initial) }`,
    probe: RESTORED,
  },
  {
    name: 'SS-044 should mask inside iframe, and remove the mask after',
    path: '/framed.html',
    source: `const inner = page.frameLocator('iframe').locator('#inner')
    await inner.waitFor()
    return describeImage(await page.screenshot({ mask: [inner] }))`,
    probe: 'window.__frameMutations',
    expectProbe: ['+glass', '-glass'],
  },
  {
    name: 'SS-008 should throw on clip outside the viewport, writing nothing',
    path: '/grid.html',
    source: `try {
      await page.screenshot({ path: at('none.png'), clip: { x: 50, y: 5000, width: 100, height: 100 } })
      return 'taken'
    } catch (error) { return errorKind(error) }`,
    noFile: 'none.png',
  },
  {
    name: 'SS-009 should throw on a negative clip size',
    path: '/grid.html',
    source: `try {
      await page.screenshot({ clip: { x: 50, y: 50, width: -100, height: 100 } })
      return 'taken'
    } catch (error) { return errorKind(error) }`,
  },
  {
    name: 'SS-033 quality option should throw for png',
    path: '/grid.html',
    source: `try { await page.screenshot({ quality: 10 }); return 'taken' } catch (error) { return errorKind(error) }`,
  },
  {
    name: 'SS-032 path option should throw for unsupported mime type',
    path: '/grid.html',
    source: `try {
      await page.screenshot({ path: at('file.txt') })
      return 'taken'
    } catch (error) { return errorKind(error) }`,
    noFile: 'file.txt',
  },
  {
    name: 'a screenshot that times out after preparing still restores the page',
    path: '/bodiless.html',
    source: `try {
      await page.screenshot({ fullPage: true, timeout: 1000, style: 'p { color: red }' })
      return 'taken'
    } catch (error) { return errorKind(error) }`,
    probe:
      "({ added: window.__stylesAdded, left: document.querySelectorAll('style').length })",
    expect: { error: { name: 'TimeoutError', strict: false } },
    expectProbe: { added: 1, left: 0 },
  },
  {
    name: 'fullPage waits for a body and times out without one',
    path: '/bodiless.html',
    source: `try {
      await page.screenshot({ fullPage: true, timeout: 1000 })
      return 'taken'
    } catch (error) { return errorKind(error) }`,
    probe: 'document.body === null',
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

/** What a script saw: its value's facts, or the kind of error that ended it. */
function seen(engine) {
  return engine.error ? { error: engine.error } : { value: facts(engine.value) }
}

/** Each engine's `file` exists and holds exactly the bytes its script returned. */
function assertWritten(side, name, engine) {
  const written = engine.file(name)
  assert.ok(written, `${side} wrote ${name}`)
  assert.equal(
    fileChecksum(written),
    engine.value.checksum,
    `${side}'s ${name} holds the returned bytes`,
  )
}

test('page.screenshot reads the same through Neo and Playwright', {
  skip,
  timeout: 600_000,
}, async (t) => {
  const pages = {
    '/grid.html': grid,
    '/transparent.html': transparent,
    '/prepared.html': prepared,
    '/bodiless.html': bodiless,
    '/framed.html': framed,
  }
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
        if (c.expectProbe)
          assert.deepEqual(
            neo.probe,
            c.expectProbe,
            'the page saw what the case names',
          )
        if (c.file) {
          assertWritten('Playwright', c.file, playwright)
          assertWritten('Neo', c.file, neo)
          assert.deepEqual(
            neo.outcome.artifacts,
            [{ path: c.file, bytes: Number(neo.value.checksum.split(':')[1]) }],
            "Neo's outcome lists the file",
          )
        }
        if (c.noFile) {
          assert.equal(
            playwright.file(c.noFile),
            null,
            'Playwright wrote nothing',
          )
          assert.equal(neo.file(c.noFile), null, 'Neo wrote nothing')
        }
        assert.deepEqual(neo.probe, playwright.probe, 'what the page kept')
      })
    }
  })
})

test('scale: css and device follow the real device scale factor', {
  skip,
  timeout: 300_000,
}, async (t) => {
  // SS-074/SS-076: a 2x device. Emulating the factor over CDP leaves the content size alone,
  // so the browser itself runs at 2x (probes-bin.md B1-scale).
  await withEngines(
    t,
    {
      pages: { '/grid.html': grid },
      browserArgs: ['--force-device-scale-factor=2'],
    },
    async (run) => {
      const { playwright, neo } = await run(
        '/grid.html',
        `const clip = { x: 50, y: 100, width: 150, height: 100 }
        return {
          device: describeImage(await page.screenshot({ clip })),
          css: describeImage(await page.screenshot({ clip, scale: 'css' })),
          fullCss: describeImage(await page.screenshot({ fullPage: true, scale: 'css' })),
        }`,
      )
      t.diagnostic(`scale: ${JSON.stringify(facts(neo.value))}`)
      assert.deepEqual(
        facts(neo.value),
        facts(playwright.value),
        'decoded sizes',
      )
      assert.equal(neo.value.device.width, 300, 'device pixels at 2x')
      assert.equal(neo.value.css.width, 150, 'CSS pixels')
    },
  )
})
