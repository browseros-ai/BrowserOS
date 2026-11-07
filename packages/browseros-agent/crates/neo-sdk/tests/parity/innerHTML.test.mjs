// innerHTML waits for the first match and returns its `innerHTML` (Playwright's `innerHTML`
// over `_waitForFunctionOnSelector`): nested markup with a comment, escaped entities and
// attributes, an empty element (''), an SVG element (which innerText refuses), a hidden one, an
// element inserted 500 ms after the `#later` click, and rejections by kind: a strict match, a
// missing element at its timeout. Cases after `LO-060 innerHTML should work`
// (tests/page/locator-convenience.spec.ts at 26137face).
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import {
  assertDecoysUntouched,
  compareFrameCases,
  failed,
  value,
} from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

test('innerHTML returns the markup of the first match and rejects ambiguous or missing targets', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/element-html.html', import.meta.url),
      'utf8',
    ),
    source: `
      const values = {
        nested: await page.innerHTML('#nested', null),
        entities: await page.mainFrame().innerHTML('#entities', null),
        attrs: await page.locator('#attrs').innerHTML(),
        empty: await page.locator('#empty').innerHTML(),
        svg: await page.locator('#chart').innerHTML(),
        hidden: await page.innerHTML('#hidden'),
        list: await page.innerHTML('#list'),
      }
      await page.click('#later')
      values.late = await page.locator('#late').innerHTML()
      const errors = {}
      for (const [name, read] of Object.entries({
        strict: () => page.locator('#list > li').innerHTML({ timeout: 1000 }),
        missing: () => page.locator('#missing').innerHTML({ timeout: 500 }),
      })) {
        try { await read(); errors[name] = 'resolved' } catch (error) { errors[name] = errorCategory(error) }
      }
      return { values, types: Object.values(values).map(v => typeof v), errors }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value, {
    values: {
      nested: 'Hello <b class="x">bold</b><br><!-- a comment --><i>it</i>',
      entities: 'Fish &amp; chips &lt;3&nbsp;"quoted"',
      attrs: '<a href="/x?a=1&amp;b=2" title="say &quot;hi&quot;">link</a>',
      empty: '',
      svg: '<text id="label" x="0" y="16">Chart label</text>',
      hidden: '<span>Hidden</span> text',
      list: '<li>First</li><li>Second</li>',
      late: '<em>late</em> arrival',
    },
    types: Array(8).fill('string'),
    errors: { strict: 'strict', missing: 'timeout' },
  })
  const reads = outcome.calls.filter((c) => c.method === 'innerHTML')
  assert.equal(reads.length, 10)
  assert.ok(reads.every((c) => c.delivery === undefined))
})

/** innerHTML through frame locators, on tabs of frame-top.html (frames.mjs): the markup of the
 * frame a locator names, cross-site and nested, and a hop that matches several iframes refused
 * as a strict mode violation. */
const frameCases = [
  {
    name: 'innerHTML reads the markup of the frame it names',
    source: `return {
      cross: await settle(() => page.frameLocator('#login-iframe').locator('#who').innerHTML()),
      nested: await settle(() => page.frameLocator('#chain').frameLocator('#chain-inner').locator('#tip').innerHTML()),
      strict: await settle(() => page.frameLocator('.slot').locator('#who').innerHTML({ timeout: 5000 })),
    }`,
    expected: {
      cross: value('cross'),
      nested: value('tip chain-inner'),
      strict: failed('strict'),
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
]

test('innerHTML inside frames reads the same markup through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
