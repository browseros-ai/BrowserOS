// inputValue is a live read on Page, Frame, Locator and label targets.
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

test('inputValue returns strings and rejects wrong or ambiguous targets', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/form-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      const values = [await page.inputValue('#text', null), await page.mainFrame().inputValue('#area', null),
        await page.locator('#single').inputValue(), await page.locator('label').inputValue()]
      if (values.length !== 4 || !values.every(v => typeof v === 'string')) throw new Error('inputValue must return strings')
      const errors = []
      for (const selector of ['#plain', '.duplicate']) {
        try { await page.locator(selector).inputValue(); errors.push('resolved') }
        catch (error) { errors.push(errorCategory(error)) }
      }
      if (errors.length !== 2 || errors[0] !== 'error' || errors[1] !== 'strict') throw new Error('wrong and strict targets must reject with their own categories')
      let invalid = 'resolved'
      try { await page.inputValue(123) } catch (error) { invalid = errorCategory(error) }
      if (invalid !== 'error') throw new Error('invalid selector type must reject')
      let timeout = 'resolved'
      try { await page.locator('#missing').inputValue({timeout:500}) } catch (error) { timeout = errorCategory(error) }
      if (timeout !== 'timeout') throw new Error('missing inputValue must time out')
      return { values, types: values.map(v => typeof v), errors, invalid, timeout }
    `,
    probe: 'window.__facts()',
  })
  assert.equal(outcome.calls.length, 8)
  assert.ok(outcome.calls.every((c) => c.delivery === undefined))
})

/** inputValue inside frames, on tabs of frame-top.html (frames.mjs): every Email holds its own
 * frame's seed, so a read routed to another frame returns the wrong one. */
const frameCases = [
  {
    name: 'inputValue reads the input of the frame it names',
    source: `const login = page.frameLocator('#login-iframe')
    return {
      same: await settle(() => page.frameLocator('#same').getByLabel('Email').inputValue()),
      cross: await settle(() => login.locator('#email').inputValue()),
      frame: await settle(() => page.frame('chain-inner').inputValue('#email')),
      select: await settle(() => login.frameLocator('#grandchild').locator('#size').inputValue()),
      notAnInput: await settle(() => page.frameLocator('#same').locator('#who').inputValue()),
      strict: await settle(() => page.frameLocator('.slot').locator('#email').inputValue()),
    }`,
    expected: {
      same: value('same@seed.test'),
      cross: value('cross@seed.test'),
      frame: value('chain-inner@seed.test'),
      select: value('s'),
      notAnInput: failed('error'),
      strict: failed('strict'),
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
]

test('inputValue inside frames reads the same input through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
