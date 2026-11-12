/** Keyboard/mouse calls pin their aligned flat JSON bytes, including nested option order. */
import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { loadPrelude, SEED } from './support/prelude-runtime'

it('pins flat input bytes for full options, empty options and the dblclick projection', async () => {
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
  // The aligned recording preserves every value while lifting only the six bags. Text pins key-order
  // drift as well as renamed fields; ignored script keys must never reach Rust.
  const recording = readFileSync(
    new URL('./data/input-wire.json', import.meta.url),
    'utf8',
  )
  expect(`${JSON.stringify(runtime.calls, null, 2)}\n`).toBe(recording)
})

it('does not read inherited option fields when the script omits their values', async () => {
  const runtime = loadPrelude({ seed: SEED, answer: () => null })
  const reads = await runtime.run(`
    let reads = 0;
    for (const key of ['delay', 'steps', 'button', 'clickCount'])
      Object.defineProperty(Object.prototype, key, { configurable: true, get() { reads++; return 7; } });
    await page.keyboard.press('a', { delay: undefined });
    await page.keyboard.type('a', { delay: undefined });
    await page.mouse.move(1, 2, { steps: undefined });
    await page.mouse.down({ button: undefined, clickCount: undefined });
    await page.mouse.up({ button: undefined, clickCount: undefined });
    await page.mouse.click(1, 2, { button: undefined, clickCount: undefined, delay: undefined });
    return reads;
  `)
  expect(reads).toBe(0)
  expect(runtime.calls).toHaveLength(6)
  for (const call of runtime.calls)
    expect(
      ['delay', 'steps', 'button', 'clickCount'].filter((key) =>
        Object.hasOwn(call.input, key),
      ),
    ).toEqual([])
})
