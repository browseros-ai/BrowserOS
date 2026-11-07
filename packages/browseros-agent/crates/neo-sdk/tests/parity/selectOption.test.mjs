// selectOption compares value/label/index matching, arrays, deselection and argument rejection.
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

test('selectOption returns selected values for every receiver', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/form-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      const values = [await page.selectOption('#single', 'two', null),
        await page.mainFrame().selectOption('#single', { label: 'First' }, null),
        await page.locator('#single').selectOption({ value: 'one' }),
        await page.locator('#multiple').selectOption(['three', 'one']),
        await page.locator('#single').selectOption(null),
        await page.locator('#single').selectOption({ index: 1 })]
      if (values.length !== 6 || !values.every(v => Array.isArray(v) && v.every(s => typeof s === 'string')))
        throw new Error('selectOption must return string arrays')
      const invalid = []
      for (const option of [{index:'bad'}, {index:NaN}, {label:null}]) {
        try { await page.locator('#single').selectOption(option); invalid.push('resolved') }
        catch (error) { invalid.push(errorCategory(error)) }
      }
      if (invalid.length !== 3 || !invalid.every(category => category === 'Error')) throw new Error('invalid option fields must reject')
      let timeout = 'resolved'
      try { await page.locator('#single').selectOption('missing', {timeout:500}) } catch (error) { timeout = errorCategory(error) }
      if (timeout !== 'TimeoutError') throw new Error('missing option must time out')
      const unchanged = await page.inputValue('#single')
      if (unchanged !== 'two') throw new Error('refused selections must leave the page unchanged')
      return { values, invalid, timeout, unchanged }
    `,
    probe: 'window.__facts()',
  })
  const delivered = outcome.calls.filter(
    (c) => c.method === 'selectOption' && c.result.kind === 'ok',
  )
  assert.equal(delivered.length, 6)
  assert.ok(delivered.every((c) => c.delivery === 'done'))
})

/** ElementHandle values: Playwright sends them as `elements` and selects the very option nodes
 * they name; Neo refuses them before anything is sent (Deviation `ELEMENT_VALUES_UNSUPPORTED`),
 * where they once travelled as empty option objects that matched the first option. A handle after
 * a string fails Playwright's validator, and Neo's refusal alike. */
test('an ElementHandle value: Playwright selects that option, Neo refuses before any effect', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/form-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      const settle = async (call) => {
        try { return { value: await call() } } catch (error) { return { error: errorCategory(error) } }
      }
      const second = await page.$('#single option:nth-child(2)')
      const two = await page.$('#multiple option:nth-child(2)')
      const three = await page.$('#multiple option:nth-child(3)')
      return {
        page: await settle(() => page.selectOption('#single', second)),
        frame: await settle(() => page.mainFrame().selectOption('#single', [second])),
        locator: await settle(() => page.locator('#multiple').selectOption([two, three])),
        mixed: await settle(() => page.locator('#single').selectOption(['one', second])),
        single: await page.inputValue('#single'),
        multiple: await page.$eval('#multiple', (s) => [...s.selectedOptions].map((o) => o.value)),
      }
    `,
    probe: 'window.__facts()',
    values: ({ playwright, neo }) => {
      assert.deepEqual(playwright, {
        page: { value: ['two'] },
        frame: { value: ['two'] },
        locator: { value: ['two', 'three'] },
        mixed: { error: 'Error' },
        single: 'two',
        multiple: ['two', 'three'],
      })
      assert.deepEqual(neo, {
        page: { error: 'Error' },
        frame: { error: 'Error' },
        locator: { error: 'Error' },
        mixed: { error: 'Error' },
        single: 'one',
        multiple: [],
      })
    },
    effects: ({ playwright, neo }) => {
      assert.ok(
        playwright.events.some((event) => event.type === 'change'),
        'Playwright selected options',
      )
      assert.deepEqual(neo.events, [], 'Neo sent nothing to the page')
      assert.deepEqual(neo.selected, [])
      assert.equal(neo.values.single, 'one')
    },
  })
})

/** selectOption inside frames, on tabs of frame-top.html (frames.mjs): by value, label and
 * index, through a frame locator, a nested one and a child Frame; a refused option leaves its
 * frame unchanged. */
const frameCases = [
  {
    name: "selectOption picks in one frame's Size and no decoy",
    source: `const login = page.frameLocator('#login-iframe')
    return {
      cross: await settle(() => login.getByLabel('Size').selectOption('m')),
      grandchild: await settle(() => login.frameLocator('#grandchild').locator('#size').selectOption({ label: 'Large' })),
      frame: await settle(() => page.frame('chain').selectOption('#size', { index: 1 })),
      missingOption: await settle(() => page.frameLocator('#same').locator('#size').selectOption('xl', { timeout: 500 })),
    }`,
    expected: {
      cross: value(['m']),
      grandchild: value(['l']),
      frame: value(['m']),
      missingOption: failed('timeout'),
    },
    effect: (effect) => {
      const { cross, grandchild, chain } = assertDecoysUntouched(effect, [
        'cross',
        'grandchild',
        'chain',
      ])
      assert.deepEqual(
        [cross.size, grandchild.size, chain.size],
        ['m', 'l', 'm'],
      )
    },
    deliveries: { selectOption: ['done', 'done', 'done', 'notSent'] },
  },
]

test('selectOption inside frames selects the same option through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
