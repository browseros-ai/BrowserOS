/**
 * `page.keyboard`, `page.mouse` and `page.touchscreen` send Playwright's own Page commands
 * (`client/input.ts` at 26137face): to the page's guid, with no timeout (`kNoTimeout`), only the
 * schema's keys, `mouse.dblclick` as a `mouseClick` of two; the getters are synchronous and
 * return the same object every time; an error names the public method.
 */

import { expect, test } from 'bun:test'
import { failure, start } from './fake-runtime'

test('tests that every device method sends its Page command with Playwright params', async () => {
  const { page, fake } = start()
  page.setDefaultTimeout(123)
  const voids = [
    await page.keyboard.down('Shift'),
    await page.keyboard.up('Shift'),
    await page.keyboard.insertText('héllo'),
    await page.keyboard.type('ab', { delay: 5, unknown: 1 } as never),
    await page.keyboard.press('Control+A', { delay: 7 }),
    await page.mouse.move(1, 2, { steps: 3 }),
    await page.mouse.down({ button: 'right', clickCount: 2 }),
    await page.mouse.up(),
    await page.mouse.click(4, 5, {
      button: 'middle',
      delay: 1,
      steps: 9,
    } as never),
    await page.mouse.dblclick(6, 7, { delay: 2 }),
    await page.mouse.wheel(0, 120),
    await page.touchscreen.tap(8, 9),
  ]
  expect(voids.every((value) => value === undefined)).toBe(true)
  expect(
    fake.messages.map(({ guid, method, params, metadata }) => [
      guid,
      method,
      params,
      (metadata as { timeout: number }).timeout,
    ]),
  ).toEqual([
    ['page@RUN', 'keyboardDown', { key: 'Shift' }, 0],
    ['page@RUN', 'keyboardUp', { key: 'Shift' }, 0],
    ['page@RUN', 'keyboardInsertText', { text: 'héllo' }, 0],
    ['page@RUN', 'keyboardType', { text: 'ab', delay: 5 }, 0],
    ['page@RUN', 'keyboardPress', { key: 'Control+A', delay: 7 }, 0],
    ['page@RUN', 'mouseMove', { x: 1, y: 2, steps: 3 }, 0],
    ['page@RUN', 'mouseDown', { button: 'right', clickCount: 2 }, 0],
    ['page@RUN', 'mouseUp', {}, 0],
    ['page@RUN', 'mouseClick', { x: 4, y: 5, delay: 1, button: 'middle' }, 0],
    ['page@RUN', 'mouseClick', { x: 6, y: 7, delay: 2, clickCount: 2 }, 0],
    ['page@RUN', 'mouseWheel', { deltaX: 0, deltaY: 120 }, 0],
    ['page@RUN', 'touchscreenTap', { x: 8, y: 9 }, 0],
  ])
})

test('tests that the device getters are synchronous and stable', () => {
  const { page } = start()
  expect(page.keyboard).toBe(page.keyboard)
  expect(page.mouse).toBe(page.mouse)
  expect(page.touchscreen).toBe(page.touchscreen)
  expect(page.keyboard).not.toBeInstanceOf(Promise)
})

test('tests that a failed device call names its public method', async () => {
  const { page, fake } = start()
  fake.answer = () => failure('Error', 'Unknown key: "Foo"')
  for (const [name, call] of [
    ['keyboard.down', () => page.keyboard.down('Foo')],
    ['mouse.dblclick', () => page.mouse.dblclick(1, 1)],
    ['touchscreen.tap', () => page.touchscreen.tap(1, 1)],
  ] as const) {
    const error = await call().then(
      () => null,
      (error: Error) => error,
    )
    expect(error?.name).toBe('Error')
    expect(error?.message.startsWith(`${name}: `)).toBe(true)
  }
})
