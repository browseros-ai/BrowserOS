/**
 * `locator.dragTo`, `page.dragAndDrop` and `frame.dragAndDrop` send Playwright's own
 * `Frame.dragAndDrop` (`client/locator.ts` at 26137face: `dragTo` passes both selectors to its
 * frame, strict): the schema's keys only, the action timeout, and an error named by the public
 * method.
 */

import { expect, test } from 'bun:test'
import { failure, start } from './fake-runtime'

test('tests that every drag method sends Frame.dragAndDrop with Playwright params', async () => {
  const { page, fake } = start()
  page.setDefaultTimeout(4321)
  const voids = [
    await page.locator('#a').dragTo(page.locator('#b'), {
      steps: 3,
      targetPosition: { x: 1, y: 2 },
      unknown: 1,
    } as never),
    await page.dragAndDrop('#a', '#b', { timeout: 10, trial: true }),
    await page
      .mainFrame()
      .dragAndDrop('#a', '#b', { sourcePosition: { x: 3, y: 4 } }),
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
      'dragAndDrop',
      {
        source: '#a',
        target: '#b',
        targetPosition: { x: 1, y: 2 },
        strict: true,
        steps: 3,
      },
      4321,
    ],
    [
      'frame@RUN',
      'dragAndDrop',
      { source: '#a', target: '#b', trial: true },
      10,
    ],
    [
      'frame@RUN',
      'dragAndDrop',
      { source: '#a', target: '#b', sourcePosition: { x: 3, y: 4 } },
      4321,
    ],
  ])
})

test('tests that a failed drag names its public method', async () => {
  const { page, fake } = start()
  fake.answer = () => failure('TimeoutError', 'Timeout 1ms exceeded.')
  for (const [name, call] of [
    ['locator.dragTo', () => page.locator('#a').dragTo(page.locator('#b'))],
    ['page.dragAndDrop', () => page.dragAndDrop('#a', '#b')],
    ['frame.dragAndDrop', () => page.mainFrame().dragAndDrop('#a', '#b')],
  ] as const) {
    const error = await call().then(
      () => null,
      (error: Error) => error,
    )
    expect(error?.name).toBe('TimeoutError')
    expect(error?.message.startsWith(`${name}: `)).toBe(true)
  }
})
