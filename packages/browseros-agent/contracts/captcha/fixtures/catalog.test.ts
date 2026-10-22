import { expect, test } from 'bun:test'
import { runInNewContext } from 'node:vm'
import {
  type CaptchaFamily,
  DEMO_PAGES,
  FIXTURE_PAGES,
  tokenLengthExpression,
} from './catalog'

test('tests that always-pass Turnstile pages also accept a widget that already passed', () => {
  const pages = [...FIXTURE_PAGES, ...DEMO_PAGES]
  for (const id of ['turnstile-pass', 'cloudflare-turnstile-demo'])
    expect(pages.find((page) => page.id === id)?.expect).toEqual([
      'solved',
      'already_clear',
    ])
})

test('tests that the catalog contains every required page with unique ids and existing paths', async () => {
  expect(FIXTURE_PAGES.map((page) => page.id)).toEqual([
    'recaptcha-v2-test',
    'turnstile-pass',
    'turnstile-block',
    'turnstile-interactive',
    'no-captcha',
    'hcaptcha-test',
  ])
  const pages = [...FIXTURE_PAGES, ...DEMO_PAGES]
  expect(new Set(pages.map((page) => page.id)).size).toBe(pages.length)
  expect(DEMO_PAGES.map((page) => page.id)).toEqual(
    expect.arrayContaining([
      'google-recaptcha-demo',
      'cloudflare-turnstile-demo',
      'hcaptcha-demo',
      '2captcha-recaptcha-demo',
      '2captcha-turnstile-demo',
    ]),
  )
  for (const page of FIXTURE_PAGES) {
    expect(
      await Bun.file(`${import.meta.dir}/pages${page.path}`).exists(),
    ).toBe(true)
    expect(page.verdict).toEqual({
      submitSelector: '#submit',
      acceptedPattern: '^ACCEPTED',
      rejectedPattern: '^REJECTED',
    })
  }
})

test.each(['recaptcha_v2', 'turnstile', 'hcaptcha', 'none'] as CaptchaFamily[])(
  'tests that %s token length reflects the real response field',
  (family) => {
    let selector = ''
    const document = {
      querySelector: (value: string) => {
        selector = value
        return { value: 'token' }
      },
    }
    expect(runInNewContext(tokenLengthExpression(family), { document })).toBe(
      family === 'none' ? 0 : 5,
    )
    const field = {
      recaptcha_v2: 'g-recaptcha-response',
      turnstile: 'cf-turnstile-response',
      hcaptcha: 'h-captcha-response',
      none: '',
    }[family]
    if (field) expect(selector).toContain(field)
    expect(
      runInNewContext(tokenLengthExpression(family), {
        document: { querySelector: () => null },
      }),
    ).toBe(0)
  },
)

test('tests that standalone pages mirror their lifecycle state and wait for visible iframes', async () => {
  for (const page of FIXTURE_PAGES) {
    const html = await Bun.file(`${import.meta.dir}/pages${page.path}`).text()
    expect(html.match(/<form\b/g)).toHaveLength(1)
    expect(html).toContain('id="captcha-form"')
    expect(html).toContain('id="submit" type="submit"')
    let visible = false
    let observe = () => {}
    let resize = () => {}
    let callbacks: {
      sitekey?: string
      callback?: (token: string) => void
      'expired-callback'?: () => void
      'error-callback'?: (code: string) => void
    } = {}
    const dataset: Record<string, string> = {}
    const document = {
      documentElement: { dataset },
      querySelector: (selector: string) =>
        visible && (page.family !== 'turnstile' || selector.includes('> div'))
          ? { getBoundingClientRect: () => ({ width: 300, height: 65 }) }
          : null,
    }
    const window: Record<string, unknown> = {}
    const render = (target: string, options: typeof callbacks) => {
      expect(target).toBe(page.family === 'turnstile' ? '#widget' : 'widget')
      callbacks = options
    }
    const context = {
      window,
      document,
      grecaptcha: { render },
      turnstile: { render },
      hcaptcha: { render },
      MutationObserver: class {
        constructor(callback: () => void) {
          observe = callback
        }
        observe() {}
        disconnect() {}
      },
      ResizeObserver: class {
        constructor(callback: () => void) {
          resize = callback
        }
        observe() {}
        disconnect() {}
      },
      // Readiness must not wait for animation frames in a background tab.
      requestAnimationFrame: () => {},
    }
    for (const match of html.matchAll(/<script>([\s\S]*?)<\/script>/g))
      runInNewContext(match[1] ?? '', context)
    expect(JSON.parse(dataset.captchaFixture ?? '{}')).toMatchObject({
      id: page.id,
      family: page.family,
      sitekey: page.sitekey ?? null,
      rendered: page.family === 'none',
      callbackFired: false,
      tokenLength: 0,
      errors: [],
    })
    expect(window.__captchaFixture).toEqual(
      JSON.parse(dataset.captchaFixture ?? '{}'),
    )
    expect(runInNewContext(page.ready, context)).toBe(page.family === 'none')
    if (page.family === 'none') continue
    ;(window.fixtureLoaded as () => void)()
    expect(callbacks.sitekey).toBe(page.sitekey)
    expect(JSON.parse(dataset.captchaFixture ?? '{}').rendered).toBe(false)
    visible = true
    if (page.family === 'turnstile') resize()
    else observe()
    expect(runInNewContext(page.ready, context)).toBe(true)
    callbacks.callback?.('fake-token')
    expect(JSON.parse(dataset.captchaFixture ?? '{}')).toMatchObject({
      callbackFired: true,
      tokenLength: 10,
    })
    callbacks['expired-callback']?.()
    expect(JSON.parse(dataset.captchaFixture ?? '{}').tokenLength).toBe(0)
    callbacks['error-callback']?.('test-error')
    expect(JSON.parse(dataset.captchaFixture ?? '{}').errors).toContain(
      'test-error',
    )
  }
})

test.each([
  {
    id: '2captcha-recaptcha-demo',
    family: 'recaptcha_v2',
    path: 'recaptcha-v2',
    sitekey: '6LfD3PIbAAAAAJs_eEHvoOl75_83eXSqpPSRFJ_u',
    mayNeedPaid: true,
    selector: '#g-recaptcha iframe',
  },
  {
    id: '2captcha-turnstile-demo',
    family: 'turnstile',
    path: 'cloudflare-turnstile',
    sitekey: '3x00000000000000000000FF',
    mayNeedPaid: false,
    selector: '#cf-turnstile > div',
  },
])(
  'tests that $id uses the live widget and independent acceptance markup',
  (expected) => {
    const page = DEMO_PAGES.find(({ id }) => id === expected.id)
    expect(page).toMatchObject({
      kind: 'demo',
      family: expected.family,
      sitekey: expected.sitekey,
      url: `https://2captcha.com/demo/${expected.path}`,
      expect: ['solved'],
      mayNeedPaid: expected.mayNeedPaid,
    })
    if (!page) throw new Error('demo missing')
    expect(page.verdict?.submitSelector).toBe('main form button[type="submit"]')
    const accepted = new RegExp(page.verdict?.acceptedPattern ?? '(?!)')
    expect(
      accepted.test('Captcha is passed successfully! {"success": true}'),
    ).toBe(true)
    expect(accepted.test('Check Reset')).toBe(false)
    expect(accepted.test('{"success": false}')).toBe(false)
    let visible = false
    const document = {
      querySelector: (selector: string) => {
        expect(selector).toBe(expected.selector)
        return visible
          ? { getBoundingClientRect: () => ({ width: 300, height: 65 }) }
          : null
      },
    }
    expect(runInNewContext(page.ready, { document })).toBe(false)
    visible = true
    expect(runInNewContext(page.ready, { document })).toBe(true)
  },
)
