// `page.coverage.startJSCoverage` and `stopJSCoverage` through Neo and through Playwright on one
// BrowserOS neo: the scripts each document ran, with every function's counts and the script's
// source, from start to stop. Anonymous (`eval`) scripts only on request, a navigation forgets
// the previous document's scripts unless `resetOnNavigation: false`, a `debugger` statement never
// stops the page, a second start fails while one runs, and a stop without a start reports
// nothing. With `resetOnNavigation: false` a left document's scripts are still gone by the
// stop, V8 having dropped them. Script ids and the fixture's origin differ per engine, so entries compare by path,
// by the type of their id, and by their functions' names and counts.
//
// Playwright cases (pin 26137face): `tests/library/chromium/js-coverage.spec.ts` "CV-001 should
// work", "CV-003 should ignore eval() scripts by default", "CV-004 shouldn't ignore eval()
// scripts if reportAnonymousScripts is true", "CV-005 should report multiple scripts", "CV-006
// should NOT report scripts across navigations when enabled" and "CV-007 should not hang when
// there is a debugger statement"; the second start fails as `server/chromium/crCoverage.ts`
// `JSCoverage.start` asserts.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const js = (body) => ({
  body,
  headers: { 'content-type': 'text/javascript' },
})

const pages = {
  '/empty.html': '<title>empty</title>',
  '/simple.html':
    '<script>\nfunction foo() {function bar() { } console.log(1); } foo(); </script>',
  '/eval.html': '<script>eval(\'console.log("foo")\')</script>',
  '/multiple.html':
    "<script src='script1.js'></script>\n<script src='script2.js'></script>",
  '/script1.js': js('console.log(3);'),
  '/script2.js': js('console.log(3);'),
}

/**
 * The page's own entries in a comparable form: path, id type, source, and each function's
 * counts. Scripts that are not the fixture's (BrowserOS's extension content scripts, which it
 * starts injecting some seconds after launch, and each engine's own anonymous scripts) differ by
 * when and where a case runs, not by engine, so they are left out.
 */
const SUMMARY = `const page_ = (entry) => entry.url.startsWith(origin) || entry.source === 'console.log("foo")'
const summary = (entries) => entries
  .filter(page_)
  .map((entry) => ({
    url: entry.url.startsWith(origin) ? entry.url.slice(origin.length) : entry.url,
    scriptId: typeof entry.scriptId,
    source: entry.source,
    functions: entry.functions.map((f) => [f.functionName, f.isBlockCoverage, f.ranges.map((r) => r.count)]),
  }))
  .sort((a, b) => a.url.localeCompare(b.url) || String(a.source).localeCompare(String(b.source)))
const origin = page.url().slice(0, page.url().indexOf('/', 'http://'.length))
`

const cases = [
  {
    name: 'reports the functions a page ran',
    source: `${SUMMARY}await page.coverage.startJSCoverage()
await page.goto(origin + '/simple.html', { waitUntil: 'load' })
return summary(await page.coverage.stopJSCoverage())`,
    check: (entries) => {
      assert.equal(entries.length, 1)
      assert.equal(entries[0].url, '/simple.html')
      assert.deepEqual(
        entries[0].functions.find(([name]) => name === 'foo')?.[2]?.[0],
        1,
      )
    },
  },
  {
    name: 'leaves eval() scripts out by default',
    source: `${SUMMARY}await page.coverage.startJSCoverage()
await page.goto(origin + '/eval.html')
return summary(await page.coverage.stopJSCoverage())`,
    check: (entries) => assert.equal(entries.length, 1),
  },
  {
    name: 'reports eval() scripts with reportAnonymousScripts',
    source: `${SUMMARY}await page.coverage.startJSCoverage({ reportAnonymousScripts: true })
await page.goto(origin + '/eval.html')
return summary(await page.coverage.stopJSCoverage()).filter((entry) => entry.url === '')`,
    check: (entries) =>
      assert.ok(entries.some((entry) => entry.source === 'console.log("foo")')),
  },
  {
    name: 'reports every script of a document',
    source: `${SUMMARY}await page.coverage.startJSCoverage()
await page.goto(origin + '/multiple.html')
return summary(await page.coverage.stopJSCoverage()).map((entry) => entry.url)`,
    check: (urls) => assert.deepEqual(urls, ['/script1.js', '/script2.js']),
  },
  {
    name: 'forgets scripts across navigations unless told not to',
    source: `${SUMMARY}await page.coverage.startJSCoverage()
await page.goto(origin + '/multiple.html')
await page.goto(origin + '/empty.html')
const reset = summary(await page.coverage.stopJSCoverage()).map((entry) => entry.url)
await page.coverage.startJSCoverage({ resetOnNavigation: false })
await page.goto(origin + '/multiple.html')
await page.goto(origin + '/empty.html')
const kept = summary(await page.coverage.stopJSCoverage()).map((entry) => entry.url)
return { reset, kept }`,
    // A left document's scripts are gone from V8 by the stop, kept or not (measured on both).
    check: ({ reset }) => assert.deepEqual(reset, []),
  },
  {
    name: 'a debugger statement does not stop the page',
    source: `${SUMMARY}await page.coverage.startJSCoverage()
await page.goto(origin + '/empty.html')
const ran = await page.evaluate(() => { debugger; return 'ran' })
return { ran, entries: summary(await page.coverage.stopJSCoverage()).length }`,
    check: ({ ran }) => assert.equal(ran, 'ran'),
  },
  {
    name: 'a second start fails while one runs; a stop without a start reports nothing',
    source: `await page.coverage.startJSCoverage()
const again = await page.coverage.startJSCoverage().then(() => 'started', errorCategory)
await page.coverage.stopJSCoverage()
return { again, unstarted: await page.coverage.stopJSCoverage() }`,
    check: ({ again, unstarted }) => {
      assert.equal(again, 'Error')
      assert.deepEqual(unstarted, [])
    },
  },
]

test('page.coverage JS: Neo matches Playwright', { skip }, async (t) => {
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
