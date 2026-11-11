/** Keyboard/mouse calls retain the baseline JSON bytes, including nested option order. */
import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { loadPrelude, SEED } from './support/prelude-runtime'

it('keeps input bytes for full options, empty options and the dblclick projection', async () => {
  const runtime = loadPrelude({ seed: SEED, answer: () => null })
  await runtime.run(`
    await page.keyboard.down('Shift')
    await page.keyboard.up('Shift')
    await page.keyboard.press('ControlOrMeta+A', { delay: 3, ignored: true })
    await page.keyboard.type('a😀', { delay: 4, ignored: true })
    await page.keyboard.insertText('b😀')
    await page.mouse.move(1.5, -2, { steps: 3, ignored: true })
    await page.mouse.down({ button: 'right', clickCount: 2, ignored: true })
    await page.mouse.up({ button: 'right', clickCount: 2, ignored: true })
    await page.mouse.click(3, 4, { button: 'middle', clickCount: 3, delay: 5, ignored: true })
    await page.mouse.dblclick(5, 6, { button: 'left', delay: 6, ignored: true })
    await page.mouse.wheel(-7, 8)
    await page.keyboard.press('a')
    await page.mouse.down()
    await page.mouse.move(9, 10)
  `)
  // Captured from the pre-migration prelude. Comparing serialized text catches key-order
  // drift as well as renamed fields; ignored script keys must never reach Rust.
  const baseline = readFileSync(
    new URL('./data/input-wire.json', import.meta.url),
    'utf8',
  )
  expect(`${JSON.stringify(runtime.calls, null, 2)}\n`).toBe(baseline)
})
