/**
 * `page.close()` and `page.bringToFront()` against a fake `__neo_runtime` (fake-runtime.ts):
 * the exact messages of the wire contract, and which failures each method swallows, as
 * Playwright's client does (`client/page.ts` at 26137face).
 */

import { describe, expect, test } from 'bun:test'
import { isTargetClosedError } from '../../src/errors'
import {
  failure,
  NO_TIMEOUT,
  OTHER_TAB,
  RUN_TAB,
  start,
  TARGET_CLOSED,
} from './fake-runtime'

const CLOSED_MESSAGE = 'Target page, context or browser has been closed'

/** What a call's rejection carries for a script, `null` when it resolved. */
async function rejection(call: Promise<unknown>) {
  return await call.then(
    () => null,
    (error) => ({
      name: error.name,
      message: error.message,
      targetClosed: isTargetClosedError(error),
    }),
  )
}

describe('Page.close', () => {
  test('closes its tab with no params and no timeout, then reads as closed', async () => {
    const { fake, page, context } = start()
    context.setDefaultTimeout(5000)
    page.setDefaultTimeout(1000)
    expect(await page.close()).toBeUndefined()
    expect(fake.messages).toEqual([
      {
        id: 1,
        guid: 'page@RUN',
        method: 'close',
        params: {},
        metadata: NO_TIMEOUT,
      },
    ])
    expect(page.isClosed()).toBe(true)
    expect(page.url()).toBe(RUN_TAB.url)
  })

  test('sends a reason when given one, and no key for an undefined one', async () => {
    const { fake, page, context } = start([RUN_TAB, OTHER_TAB])
    const [, other] = context.pages()
    await other.close({ reason: 'done with it' })
    await page.close({ reason: undefined })
    expect(
      fake.messages.map(({ guid, method, params }) => ({
        guid,
        method,
        params,
      })),
    ).toEqual([
      { guid: 'page@B', method: 'close', params: { reason: 'done with it' } },
      { guid: 'page@RUN', method: 'close', params: {} },
    ])
  })

  test('runBeforeUnload sends runBeforeUnload instead, for any truthy value, as Playwright', async () => {
    const { fake, page, context } = start([RUN_TAB, OTHER_TAB])
    const [, other] = context.pages()
    expect(await page.close({ runBeforeUnload: true })).toBeUndefined()
    // Playwright tests the option for truth, so a script's 'x' runs beforeunload too.
    await other.close({
      runBeforeUnload: 'x' as unknown as boolean,
      reason: 'ignored',
    })
    expect(fake.messages).toEqual([
      {
        id: 1,
        guid: 'page@RUN',
        method: 'runBeforeUnload',
        params: {},
        metadata: NO_TIMEOUT,
      },
      {
        id: 2,
        guid: 'page@B',
        method: 'runBeforeUnload',
        params: {},
        metadata: NO_TIMEOUT,
      },
    ])
  })

  test('closing a closed page resolves: close swallows TargetClosedError', async () => {
    const { fake, page } = start()
    fake.answer = () => TARGET_CLOSED
    expect(await rejection(page.close())).toBeNull()
    expect(await rejection(page.close({ reason: 'again' }))).toBeNull()
  })

  test('runBeforeUnload on a closed page rejects: only close swallows it', async () => {
    const { fake, page } = start()
    fake.tabs = []
    expect(await rejection(page.close({ runBeforeUnload: true }))).toEqual({
      name: 'Error',
      message: `page.close: ${CLOSED_MESSAGE}`,
      targetClosed: true,
    })
  })

  test('close rethrows every other failure', async () => {
    const { fake, page } = start()
    fake.answer = () => failure('Error', 'invalid params for Page.close')
    expect(await rejection(page.close())).toEqual({
      name: 'Error',
      message: 'page.close: invalid params for Page.close',
      targetClosed: false,
    })
    fake.answer = () => failure('TimeoutError', 'Timeout 30000ms exceeded.')
    expect(await rejection(page.close())).toEqual({
      name: 'TimeoutError',
      message: 'page.close: Timeout 30000ms exceeded.',
      targetClosed: false,
    })
  })
})

describe('Page.bringToFront', () => {
  test('activates its own tab with no params and no timeout', async () => {
    const { fake, page, context } = start([RUN_TAB, OTHER_TAB])
    const [, other] = context.pages()
    page.setDefaultTimeout(1000)
    expect(await other.bringToFront()).toBeUndefined()
    expect(fake.messages).toEqual([
      {
        id: 1,
        guid: 'page@B',
        method: 'bringToFront',
        params: {},
        metadata: NO_TIMEOUT,
      },
    ])
    expect(fake.active).toBe('page@B')
  })

  test('rejects on a closed page with TargetClosedError, prefixed page.bringToFront', async () => {
    const { page } = start()
    await page.close()
    expect(await rejection(page.bringToFront())).toEqual({
      name: 'Error',
      message: `page.bringToFront: ${CLOSED_MESSAGE}`,
      targetClosed: true,
    })
  })
})
