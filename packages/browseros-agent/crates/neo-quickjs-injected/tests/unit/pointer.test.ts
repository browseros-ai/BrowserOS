/**
 * `dblclick`, `hover` and `tap` send Playwright's own protocol calls (`client/locator.ts`,
 * `client/frame.ts`, `client/page.ts` at 26137face): a locator's are strict, the page and frame
 * forms are not, only the schema's keys travel (the deprecated `noWaitAfter` and `timeout`
 * never do), the timeout resolves into the call's metadata, and an error names the public
 * method that failed.
 */

import { expect, test } from 'bun:test'
import { failure, start } from './fake-runtime'

const METHODS = ['dblclick', 'hover', 'tap'] as const

test('tests that every receiver sends its pointer action with Playwright params', async () => {
  const { page, fake } = start()
  page.setDefaultTimeout(321)
  for (const method of METHODS) {
    const options = {
      position: { x: 1, y: 2 },
      trial: true,
      noWaitAfter: true,
      timeout: 50,
    }
    expect(await page.locator('#go')[method](options)).toBeUndefined()
    expect(await page[method]('#go', null as never)).toBeUndefined()
    expect(
      await page.mainFrame()[method]('#go', { force: true }),
    ).toBeUndefined()
  }
  expect(
    fake.messages.map(({ guid, method, params, metadata }) => [
      guid,
      method,
      params,
      (metadata as { timeout: number }).timeout,
    ]),
  ).toEqual(
    METHODS.flatMap((method) => [
      [
        'frame@RUN',
        method,
        {
          selector: '#go',
          strict: true,
          position: { x: 1, y: 2 },
          trial: true,
        },
        50,
      ],
      ['frame@RUN', method, { selector: '#go' }, 321],
      ['frame@RUN', method, { selector: '#go', force: true }, 321],
    ]),
  )
})

test('tests that dblclick sends its button, delay and steps, and no click count', async () => {
  const { page, fake } = start()
  await page
    .locator('#go')
    .dblclick({ button: 'right', delay: 10, steps: 1, clickCount: 3 } as never)
  expect(fake.messages.map(({ params }) => params)).toEqual([
    { selector: '#go', strict: true, delay: 10, button: 'right', steps: 1 },
  ])
})

test('tests that a failed pointer action names its public method', async () => {
  const { page, fake } = start()
  fake.answer = () => failure('TimeoutError', 'Timeout 50ms exceeded.')
  for (const [name, call] of [
    ['locator.hover', () => page.locator('#go').hover({ timeout: 50 })],
    ['page.tap', () => page.tap('#go', { timeout: 50 })],
    ['frame.dblclick', () => page.mainFrame().dblclick('#go', { timeout: 50 })],
  ] as const) {
    const error = await call().then(
      () => null,
      (error: Error) => error,
    )
    expect(error?.name).toBe('TimeoutError')
    expect(error?.message.startsWith(`${name}: `)).toBe(true)
  }
})
