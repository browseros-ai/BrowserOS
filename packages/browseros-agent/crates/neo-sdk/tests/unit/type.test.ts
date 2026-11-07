/**
 * `locator.pressSequentially`, the deprecated `locator.type`, and `page.type`/`frame.type` send
 * Playwright's own `Frame.type` (`client/locator.ts` at 26137face: `pressSequentially` is
 * `type`): the locator strict, the schema's keys only, the action timeout, and an error named by
 * the public method.
 */

import { expect, test } from 'bun:test'
import { failure, start } from './fake-runtime'

test('tests that every typing method sends Frame.type with Playwright params', async () => {
  const { page, fake } = start()
  page.setDefaultTimeout(4321)
  const locator = page.locator('#name')
  const voids = [
    await locator.pressSequentially('hi', { delay: 5 }),
    await locator.type('yo', { timeout: 10, unknown: 1 } as never),
    await page.type('#name', 'ab'),
    await page.mainFrame().type('#name', 'cd', { delay: 0 }),
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
    [
      'frame@RUN',
      'type',
      { selector: '#name', strict: true, text: 'hi', delay: 5 },
      4321,
    ],
    ['frame@RUN', 'type', { selector: '#name', strict: true, text: 'yo' }, 10],
    ['frame@RUN', 'type', { selector: '#name', text: 'ab' }, 4321],
    ['frame@RUN', 'type', { selector: '#name', text: 'cd', delay: 0 }, 4321],
  ])
})

test('tests that a failed typing call names its public method', async () => {
  const { page, fake } = start()
  fake.answer = () => failure('TimeoutError', 'Timeout 1ms exceeded.')
  for (const [name, call] of [
    [
      'locator.pressSequentially',
      () => page.locator('#n').pressSequentially('x'),
    ],
    ['locator.type', () => page.locator('#n').type('x')],
    ['page.type', () => page.type('#n', 'x')],
    ['frame.type', () => page.mainFrame().type('#n', 'x')],
  ] as const) {
    const error = await call().then(
      () => null,
      (error: Error) => error,
    )
    expect(error?.name).toBe('TimeoutError')
    expect(error?.message.startsWith(`${name}: `)).toBe(true)
  }
})
