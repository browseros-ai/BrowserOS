// The harness's own promises, checked without a browser: `errorCategory`, which every
// compareMethod script reports its rejections through, tells errors apart by name.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { errorCategory } from './harness.mjs'

test('errorCategory reports the error name and whether it is a strict mode violation', () => {
  const named = (name, message) => Object.assign(new Error(message), { name })
  assert.deepEqual(
    [
      new Error('locator.click: element is not attached'),
      new TypeError("cannot read property 'timeout' of null"),
      named('TimeoutError', 'locator.click: Timeout 500ms exceeded.'),
      new Error('locator.click: Error: strict mode violation: resolved to 2'),
    ].map(errorCategory),
    ['Error', 'TypeError', 'TimeoutError', 'Error:strict'],
  )
})
