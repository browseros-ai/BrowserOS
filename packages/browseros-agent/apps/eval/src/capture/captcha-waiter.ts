import type { Browser } from '@browseros/browser-core/browser'

export interface CaptchaWaitResult {
  detected: boolean
  type: 'recaptcha' | 'hcaptcha' | 'turnstile' | 'none'
  solved: boolean
  waitDurationMs: number
}

export interface CaptchaWaiterConfig {
  waitTimeoutMs: number
  pollIntervalMs: number
}

const DETECTION_SCRIPT = `(() => {
  const recaptcha = document.querySelector('iframe[src*="recaptcha"]')
  if (recaptcha) {
    const response = document.getElementById('g-recaptcha-response')
    return { type: 'recaptcha', solved: !!(response && response.value) }
  }
  const hcaptcha = document.querySelector('iframe[src*="hcaptcha"]')
  if (hcaptcha) {
    const response = document.querySelector('[name="h-captcha-response"]')
    return { type: 'hcaptcha', solved: !!(response && response.value) }
  }
  const turnstile = document.querySelector('iframe[src*="challenges.cloudflare.com"]')
  if (turnstile) {
    const response = document.querySelector('[name="cf-turnstile-response"]')
    return { type: 'turnstile', solved: !!(response && response.value) }
  }
  return { type: 'none', solved: false }
})()`

export class CaptchaWaiter {
  private readonly config: CaptchaWaiterConfig

  constructor(config: CaptchaWaiterConfig) {
    this.config = config
  }

  async waitIfCaptchaPresent(
    browser: Browser,
    pageId: number,
  ): Promise<CaptchaWaitResult> {
    const start = Date.now()

    try {
      const initial = await this.detect(browser, pageId)
      if (initial.type === 'none') {
        return {
          detected: false,
          type: 'none',
          solved: false,
          waitDurationMs: Date.now() - start,
        }
      }

      if (initial.solved) {
        return {
          detected: true,
          type: initial.type,
          solved: true,
          waitDurationMs: Date.now() - start,
        }
      }

      // Poll until solved or timeout
      while (Date.now() - start < this.config.waitTimeoutMs) {
        await sleep(this.config.pollIntervalMs)
        const check = await this.detect(browser, pageId)
        if (check.solved || check.type === 'none') {
          return {
            detected: true,
            type: initial.type,
            solved: check.solved,
            waitDurationMs: Date.now() - start,
          }
        }
      }

      return {
        detected: true,
        type: initial.type,
        solved: false,
        waitDurationMs: Date.now() - start,
      }
    } catch {
      return {
        detected: false,
        type: 'none',
        solved: false,
        waitDurationMs: Date.now() - start,
      }
    }
  }

  private async detect(
    browser: Browser,
    pageId: number,
  ): Promise<{ type: CaptchaWaitResult['type']; solved: boolean }> {
    const result = await browser.evaluate(pageId, DETECTION_SCRIPT)
    if (result.error || !result.value) {
      return { type: 'none', solved: false }
    }
    const val = result.value as { type: string; solved: boolean }
    return {
      type: (val.type as CaptchaWaitResult['type']) ?? 'none',
      solved: val.solved ?? false,
    }
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
