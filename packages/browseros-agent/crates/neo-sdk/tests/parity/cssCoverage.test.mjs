// `page.coverage.startCSSCoverage` and `stopCSSCoverage` through Neo and through Playwright on
// one BrowserOS neo: every stylesheet a document loaded, with its text and the disjoint ranges
// its used rules cover, from start to stop. A stylesheet's `sourceURL` names it, one injected by
// script is left out, a navigation forgets the previous document's stylesheets unless
// `resetOnNavigation: false`, a second start fails while one runs, and a stop without a start
// reports nothing. CV-014's injected stylesheet is a `<style>` the page's script adds (Playwright's
// `addStyleTag` adds one the same way). Stylesheets that are not the fixture's (BrowserOS's extension content
// scripts) differ by when a case runs, not by engine, so they are left out.
//
// Playwright cases (pin 26137face): `tests/library/chromium/css-coverage.spec.ts` "CV-008
// should work", "CV-010 should report multiple stylesheets", "CV-011 should report stylesheets
// that have no coverage", "CV-014 should ignore injected stylesheets", "CV-015 should report
// stylesheets across navigations" and "CV-016 should NOT report scripts across navigations";
// the second start fails as `server/chromium/crCoverage.ts` `CSSCoverage.start` asserts.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const css = (body) => ({ body, headers: { 'content-type': 'text/css' } })

const pages = {
  '/empty.html': '<title>empty</title>',
  '/simple.html':
    '<style>\ndiv { color: green; }\na { color: blue; }\n</style>\n<div>hello, world</div>\n',
  '/unused.html':
    '<style>\n@media screen {\n  a { color: green; }\n}\n/*# sourceURL=unused.css */\n</style>\n',
  '/multiple.html':
    '<link rel="stylesheet" href="stylesheet1.css">\n<link rel="stylesheet" href="stylesheet2.css">\n<script>\nwindow.addEventListener(\'DOMContentLoaded\', () => {\n  console.log(window.getComputedStyle(document.body).color);\n}, false);\n</script>\n',
  '/stylesheet1.css': css('body {\n  color: red;\n}\n'),
  '/stylesheet2.css': css('html {\n  margin: 0;\n  padding: 0;\n}\n'),
}

/** The fixture's entries, by path (or the `sourceURL` a stylesheet names), sorted. */
const SUMMARY = `const origin = page.url().slice(0, page.url().indexOf('/', 'http://'.length))
const summary = (entries) => entries
  .filter((entry) => !entry.url.startsWith('chrome-extension://'))
  .map((entry) => ({
    url: entry.url.startsWith(origin) ? entry.url.slice(origin.length) : entry.url,
    ranges: entry.ranges,
    used: entry.ranges.map((range) => entry.text.substring(range.start, range.end)),
  }))
  .sort((a, b) => a.url.localeCompare(b.url))
`

const cases = [
  {
    name: 'reports the used rules of a stylesheet',
    source: `${SUMMARY}await page.coverage.startCSSCoverage()
await page.goto(origin + '/simple.html')
return summary(await page.coverage.stopCSSCoverage())`,
    check: (entries) =>
      assert.deepEqual(entries, [
        {
          url: '/simple.html',
          ranges: [{ start: 1, end: 22 }],
          used: ['div { color: green; }'],
        },
      ]),
  },
  {
    name: 'reports every stylesheet of a document',
    source: `${SUMMARY}await page.coverage.startCSSCoverage()
await page.goto(origin + '/multiple.html')
return summary(await page.coverage.stopCSSCoverage()).map((entry) => entry.url)`,
    check: (urls) =>
      assert.deepEqual(urls, ['/stylesheet1.css', '/stylesheet2.css']),
  },
  {
    name: 'reports a stylesheet with no coverage, by its sourceURL',
    source: `${SUMMARY}await page.coverage.startCSSCoverage()
await page.goto(origin + '/unused.html')
return summary(await page.coverage.stopCSSCoverage())`,
    check: (entries) =>
      assert.deepEqual(entries, [{ url: 'unused.css', ranges: [], used: [] }]),
  },
  {
    name: 'leaves out a stylesheet a script injected',
    source: `${SUMMARY}await page.coverage.startCSSCoverage()
await page.evaluate(() => {
  const style = document.createElement('style')
  style.textContent = 'body { margin: 10px;}'
  document.head.append(style)
})
const margin = await page.evaluate(() => window.getComputedStyle(document.body).margin)
return { margin, entries: summary(await page.coverage.stopCSSCoverage()) }`,
    check: ({ margin, entries }) => {
      assert.equal(margin, '10px')
      assert.deepEqual(entries, [])
    },
  },
  {
    name: 'forgets stylesheets across navigations unless told not to',
    source: `${SUMMARY}await page.coverage.startCSSCoverage()
await page.goto(origin + '/multiple.html')
await page.goto(origin + '/empty.html')
const reset = summary(await page.coverage.stopCSSCoverage()).map((entry) => entry.url)
await page.coverage.startCSSCoverage({ resetOnNavigation: false })
await page.goto(origin + '/multiple.html')
await page.goto(origin + '/empty.html')
const kept = summary(await page.coverage.stopCSSCoverage()).map((entry) => entry.url)
return { reset, kept }`,
    check: ({ reset, kept }) => {
      assert.deepEqual(reset, [])
      assert.deepEqual(kept, ['/stylesheet1.css', '/stylesheet2.css'])
    },
  },
  {
    name: 'a second start fails while one runs; a stop without a start reports nothing',
    source: `await page.coverage.startCSSCoverage()
const again = await page.coverage.startCSSCoverage().then(() => 'started', errorCategory)
await page.coverage.stopCSSCoverage()
return { again, unstarted: await page.coverage.stopCSSCoverage() }`,
    check: ({ again, unstarted }) => {
      assert.equal(again, 'Error')
      assert.deepEqual(unstarted, [])
    },
  },
]

test('page.coverage CSS: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    for (const { name, source, check } of cases) {
      const reference = outcomeValue(await playwright('/empty.html', source))
      check(reference)
      assert.deepEqual(
        outcomeValue(await neo('/empty.html', source)),
        reference,
        `${name}: Neo`,
      )
    }
  })
})
