import { beforeEach, describe, expect, it, mock } from 'bun:test'
import type { Browser } from '@browseros/browser-core/browser'
import { CaptchaWaiter } from '../../src/capture/captcha-waiter'

function createMockBrowser(
  evaluateResults: Array<{ value?: unknown; error?: string }>,
): Browser {
  let callIndex = 0
  return {
    evaluate: mock(async (_page: number, _expr: string) => {
      const result = evaluateResults[callIndex] ?? evaluateResults.at(-1)
      callIndex++
      return result
    }),
  } as unknown as Browser
}

describe('CaptchaWaiter', () => {
  let waiter: CaptchaWaiter

  beforeEach(() => {
    waiter = new CaptchaWaiter({
      waitTimeoutMs: 5000,
      pollIntervalMs: 100,
    })
  })

  it('returns immediately when no CAPTCHA detected', async () => {
    const browser = createMockBrowser([
      { value: { type: 'none', solved: false } },
    ])

    const result = await waiter.waitIfCaptchaPresent(browser, 1)

    expect(result.detected).toBe(false)
    expect(result.type).toBe('none')
    expect(result.solved).toBe(false)
    expect(browser.evaluate).toHaveBeenCalledTimes(1)
  })

  it('returns immediately when CAPTCHA already solved', async () => {
    const browser = createMockBrowser([
      { value: { type: 'recaptcha', solved: true } },
    ])

    const result = await waiter.waitIfCaptchaPresent(browser, 1)

    expect(result.detected).toBe(true)
    expect(result.type).toBe('recaptcha')
    expect(result.solved).toBe(true)
    expect(browser.evaluate).toHaveBeenCalledTimes(1)
  })

  it('polls until CAPTCHA is solved', async () => {
    const browser = createMockBrowser([
      { value: { type: 'hcaptcha', solved: false } },
      { value: { type: 'hcaptcha', solved: false } },
      { value: { type: 'hcaptcha', solved: true } },
    ])

    const result = await waiter.waitIfCaptchaPresent(browser, 1)

    expect(result.detected).toBe(true)
    expect(result.type).toBe('hcaptcha')
    expect(result.solved).toBe(true)
    expect(browser.evaluate).toHaveBeenCalledTimes(3)
  })

  it('polls until CAPTCHA disappears', async () => {
    const browser = createMockBrowser([
      { value: { type: 'turnstile', solved: false } },
      { value: { type: 'turnstile', solved: false } },
      { value: { type: 'none', solved: false } },
    ])

    const result = await waiter.waitIfCaptchaPresent(browser, 1)

    expect(result.detected).toBe(true)
    expect(result.type).toBe('turnstile')
    expect(result.solved).toBe(false)
    expect(browser.evaluate).toHaveBeenCalledTimes(3)
  })

  it('times out if CAPTCHA never solves', async () => {
    const shortWaiter = new CaptchaWaiter({
      waitTimeoutMs: 300,
      pollIntervalMs: 100,
    })

    const browser = createMockBrowser([
      { value: { type: 'recaptcha', solved: false } },
    ])

    const result = await shortWaiter.waitIfCaptchaPresent(browser, 1)

    expect(result.detected).toBe(true)
    expect(result.type).toBe('recaptcha')
    expect(result.solved).toBe(false)
    expect(result.waitDurationMs).toBeGreaterThanOrEqual(250)
  })

  it('handles browser.evaluate errors gracefully', async () => {
    const browser = createMockBrowser([{ error: 'Page crashed' }])

    const result = await waiter.waitIfCaptchaPresent(browser, 1)

    expect(result.detected).toBe(false)
    expect(result.type).toBe('none')
    expect(result.solved).toBe(false)
  })

  it('handles browser.evaluate throwing', async () => {
    const browser = {
      evaluate: mock(async () => {
        throw new Error('Connection lost')
      }),
    } as unknown as Browser

    const result = await waiter.waitIfCaptchaPresent(browser, 1)

    expect(result.detected).toBe(false)
    expect(result.type).toBe('none')
    expect(result.solved).toBe(false)
  })

  it('tracks wait duration', async () => {
    const browser = createMockBrowser([
      { value: { type: 'recaptcha', solved: false } },
      { value: { type: 'recaptcha', solved: false } },
      { value: { type: 'recaptcha', solved: true } },
    ])

    const result = await waiter.waitIfCaptchaPresent(browser, 1)

    expect(result.waitDurationMs).toBeGreaterThanOrEqual(150)
  })
})
