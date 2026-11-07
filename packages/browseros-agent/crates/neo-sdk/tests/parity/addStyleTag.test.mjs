// page.addStyleTag and frame.addStyleTag through the Playwright reference and through Neo on
// fresh pages: a stylesheet `<link>` from a URL or a `<style>` from content in the page's head,
// answered as an element handle once it loaded, and applied (the probe reads a computed color).
// No source, a URL that fails to load and a page whose Content Security Policy blocks the style
// are refused, and the page's style stays. Values by `describe` (values.mjs), errors by category
// and the words Playwright's tests look for. URLs are `data:` stylesheets and an unreachable port.
// `path` is refused by Neo as in addScriptTag.test.mjs.
//
// Cases from tests/page/page-add-style-tag.spec.ts at 26137face: AT-013 "should throw an error if
// no options are provided", AT-014 "should work with a url", AT-015 "should throw an error if
// loading from url fail", AT-018 "should work with content", AT-019 "should throw when added with
// content to the CSP page", AT-020 "should throw when added with URL to the CSP page".
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = '<title>styles</title><p id="p">text</p><p id="q">text</p>'
const csp = `<meta http-equiv="Content-Security-Policy" content="style-src 'self'"><title>csp</title><p id="p">text</p><p id="q">text</p>`
const probe =
  '({ p: getComputedStyle(document.querySelector("#p")).color, q: getComputedStyle(document.querySelector("#q")).color, tags: [...document.head.querySelectorAll("style, link")].map((e) => e.tagName) })'

test('addStyleTag adds a loaded stylesheet as Playwright does', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const tag = async (options) => (await page.addStyleTag(options)).evaluate((e) => e.tagName)
      return {
        none: await outcome(() => page.addStyleTag(), ['Provide an object with a']),
        url: await outcome(() => tag({ url: 'data:text/css,%23p{color:rgb(255, 0, 0)}' })),
        content: await outcome(() => tag({ content: '#q { color: rgb(0, 0, 255) }' })),
        fails: await outcome(() => page.addStyleTag({ url: 'http://127.0.0.1:1/missing.css' })),
        frameForm: await outcome(async () => (await page.mainFrame().addStyleTag({ content: 'body { margin: 3px }' })).evaluate((e) => e.textContent)),
      }
    `,
    probe,
  })
  assert.deepEqual(outcome.value.url, { value: 'LINK' })
  assert.deepEqual(outcome.value.content, { value: 'STYLE' })
})

test('a page whose Content Security Policy blocks the style refuses the tag', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: csp,
    source: `${VALUES}
      return {
        content: await outcome(() => page.addStyleTag({ content: '#q { color: rgb(0, 0, 255) }' })),
        url: await outcome(() => page.addStyleTag({ url: 'data:text/css,%23p{color:rgb(255, 0, 0)}' })),
      }
    `,
    probe,
  })
  assert.deepEqual(outcome.value, {
    content: { error: 'Error', mentions: [] },
    url: { error: 'Error', mentions: [] },
  })
})
