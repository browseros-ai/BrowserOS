/**
 * `page.clock` / `browserContext.clock` against a fake `__neo_runtime` (fake-runtime.ts): the
 * context's `clock*` messages Playwright's client sends (`client/clock.ts` at 26137face), and the
 * times and tick counts refused before any message: Playwright's client check of a `Date`, and
 * its server's of a number Neo's JSON cannot carry.
 */

import { describe, expect, test } from 'bun:test'
import { NO_TIMEOUT, start } from './fake-runtime'

const OK = { result: {}, log: [] }

describe('clock', () => {
  test('is the context’s, and sends its commands there', async () => {
    const { fake, page, context } = start()
    fake.answer = () => OK
    expect(page.clock).toBe(context.clock)
    await page.clock.install()
    await page.clock.install({ time: new Date(5) })
    await page.clock.pauseAt('2024-02-02')
    await page.clock.fastForward('01:00')
    await page.clock.runFor(5)
    await page.clock.setFixedTime(Number.NaN)
    await page.clock.resume()
    const sent: [string, object][] = [
      ['clockInstall', {}],
      ['clockInstall', { timeNumber: 5 }],
      ['clockPauseAt', { timeString: '2024-02-02' }],
      ['clockFastForward', { ticksString: '01:00' }],
      ['clockRunFor', { ticksNumber: 5 }],
      ['clockSetFixedTime', { timeNumber: 0 }],
      ['clockResume', {}],
    ]
    expect(fake.messages).toEqual(
      sent.map(([method, params], index) => ({
        id: index + 1,
        guid: 'context@default',
        method,
        params,
        metadata: NO_TIMEOUT,
      })),
    )
  })

  test('refuses before any message what it cannot send', async () => {
    const { fake, page } = start()
    fake.answer = () => OK
    await expect(page.clock.runFor(Number.NaN)).rejects.toThrow(
      'clock.runFor: Invalid ticks: NaN',
    )
    await expect(
      page.clock.fastForward(Number.POSITIVE_INFINITY),
    ).rejects.toThrow('clock.fastForward: Invalid ticks: Infinity')
    await expect(page.clock.setSystemTime(new Date('invalid'))).rejects.toThrow(
      'Invalid date: Invalid Date',
    )
    await expect(page.clock.pauseAt(Number.POSITIVE_INFINITY)).rejects.toThrow(
      'clock.pauseAt: Invalid date: Infinity',
    )
    expect(fake.messages).toEqual([])
  })
})
