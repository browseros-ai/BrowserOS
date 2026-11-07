// Page.ariaSnapshotJSON and Locator.ariaSnapshotJSON through Neo and Playwright: the tree the
// YAML snapshot renders, as Playwright's AriaSnapshotJSON (roles, names, flags, link urls, text
// fragments), in default and ai mode (refs, cursor), depth-limited, with boxes, for a page and a
// locator, compared value for value between the engines. `null` options throw Playwright's
// TypeError, a `null` or unknown mode is an Error, and a locator with no match times out. The
// fixture removes BrowserOS's takeover UI so the tree is the fixture's alone.
//
// From Playwright's specs at the pin (tests/page/page-aria-snapshot-json.spec.ts at 26137face):
// 'AR-096' to 'AR-102', 'AR-104', 'AR-105' and 'AR-106'.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const ISOLATE = `<script>new MutationObserver(() => document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement, { childList: true, subtree: true })</script>`

const HTML = `<h1>Title</h1><p>Some <b>bold</b> text</p>
<button>Go</button><button disabled>Off</button>
<label><input type="checkbox" checked> Agree</label>
<a href="https://example.com/">Link</a>
<input type="text" placeholder="Name" value="Ann">
<ul id="list"><li>One</li><li>Two</li></ul>${ISOLATE}`

test('ariaSnapshotJSON answers the snapshot tree as Playwright does', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: HTML,
    source: `
      const settled = []
      for (const options of [null, { mode: null }, { mode: 'bogus' }]) {
        try { await page.ariaSnapshotJSON(options); settled.push('resolved') } catch (error) { settled.push(errorCategory(error)) }
      }
      return {
        tree: await page.ariaSnapshotJSON(),
        ai: await page.ariaSnapshotJSON({ mode: 'ai' }),
        shallow: await page.ariaSnapshotJSON({ mode: 'ai', depth: 1 }),
        boxes: await page.ariaSnapshotJSON({ boxes: true }),
        list: await page.locator('#list').ariaSnapshotJSON(),
        listAi: await page.locator('#list').ariaSnapshotJSON({ mode: 'ai' }),
        viaFrame: await page.mainFrame().locator('#list').ariaSnapshotJSON(),
        missing: await page.locator('#none').ariaSnapshotJSON({ timeout: 500 }).then(() => 'resolved', errorCategory),
        settled,
      }
    `,
    probe: 'document.title',
  })
  const { tree, ai, list, listAi, settled, missing } = outcome.value
  assert.ok(Array.isArray(tree) && tree.length > 0, JSON.stringify(tree))
  assert.ok(
    JSON.stringify(tree).includes('"role":"heading"'),
    JSON.stringify(tree),
  )
  assert.ok(!JSON.stringify(tree).includes('"ref"'), 'no refs in default mode')
  assert.ok(JSON.stringify(ai).includes('"ref":"e'), 'ai mode names refs')
  assert.equal(list[0].role, 'list')
  assert.ok(JSON.stringify(listAi).includes('"ref"'))
  assert.equal(missing, 'TimeoutError')
  assert.deepEqual(settled, ['TypeError', 'Error', 'Error'])
})
