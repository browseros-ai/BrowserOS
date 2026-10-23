export type CaptchaFamily = 'recaptcha_v2' | 'turnstile' | 'hcaptcha' | 'none'
export type TerminalStatus =
  | 'solved'
  | 'already_clear'
  | 'unsupported'
  | 'failed'
  | 'cancelled'
  | 'needs_human'
export interface PageVerdict {
  submitSelector: string
  acceptedPattern: string
  rejectedPattern?: string
}
export interface CaptchaPage {
  id: string
  title: string
  kind: 'fixture' | 'demo'
  family: CaptchaFamily
  sitekey?: string
  path?: string
  url?: string
  expect: TerminalStatus[]
  mayNeedPaid: boolean
  ready: string
  verdict?: PageVerdict
  notes?: string
}

function fixture(
  id: string,
  title: string,
  family: CaptchaFamily,
  path: string,
  sitekey?: string,
  expect: TerminalStatus[] = ['solved'],
): CaptchaPage {
  return {
    id,
    title,
    kind: 'fixture',
    family,
    path,
    sitekey,
    expect,
    mayNeedPaid: false,
    ready:
      "JSON.parse(document.documentElement.dataset.captchaFixture || '{}').rendered === true",
    verdict: {
      submitSelector: '#submit',
      acceptedPattern: '^ACCEPTED',
      rejectedPattern: '^REJECTED',
    },
  }
}

export const FIXTURE_PAGES: readonly CaptchaPage[] = [
  fixture(
    'recaptcha-v2-test',
    'reCAPTCHA v2 test checkbox',
    'recaptcha_v2',
    '/recaptcha-v2.html',
    '6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI',
  ),
  fixture(
    'turnstile-pass',
    'Turnstile always passes',
    'turnstile',
    '/turnstile-pass.html',
    '1x00000000000000000000AA',
    ['solved', 'already_clear'],
  ),
  fixture(
    'turnstile-block',
    'Turnstile always blocks',
    'turnstile',
    '/turnstile-block.html',
    '2x00000000000000000000AB',
    ['failed', 'needs_human'],
  ),
  fixture(
    'turnstile-interactive',
    'Turnstile interactive challenge',
    'turnstile',
    '/turnstile-interactive.html',
    '3x00000000000000000000FF',
  ),
  fixture(
    'no-captcha',
    'Ordinary form',
    'none',
    '/no-captcha.html',
    undefined,
    ['already_clear'],
  ),
  fixture(
    'hcaptcha-test',
    'hCaptcha test checkbox',
    'hcaptcha',
    '/hcaptcha.html',
    '10000000-ffff-ffff-ffff-000000000001',
    ['unsupported', 'needs_human'],
  ),
]

export const DEMO_PAGES: readonly CaptchaPage[] = [
  {
    id: 'google-recaptcha-demo',
    title: 'Google reCAPTCHA demo',
    kind: 'demo',
    family: 'recaptcha_v2',
    url: 'https://www.google.com/recaptcha/api2/demo',
    sitekey: '6Le-wvkSAAAAAPBMRTvw0Q4Muexq9bi0DJwx_mJ-',
    expect: ['solved'],
    mayNeedPaid: true,
    ready: '!!document.querySelector("#recaptcha-demo iframe")',
    verdict: {
      submitSelector: '#recaptcha-demo-submit',
      acceptedPattern: 'Verification Success',
    },
  },
  {
    id: 'cloudflare-turnstile-demo',
    title: 'Cloudflare Turnstile demo',
    kind: 'demo',
    family: 'turnstile',
    url: 'https://demo.turnstile.workers.dev/',
    sitekey: '1x00000000000000000000AA',
    expect: ['solved', 'already_clear'],
    mayNeedPaid: false,
    ready:
      '(document.querySelector(".cf-turnstile > div")?.getBoundingClientRect().height ?? 0) > 0',
    verdict: {
      submitSelector: 'form[action="/handler"] button[type="submit"]',
      acceptedPattern: '"success"\\s*:\\s*true',
      rejectedPattern: '"success"\\s*:\\s*false',
    },
  },
  {
    id: 'hcaptcha-demo',
    title: 'hCaptcha demo',
    kind: 'demo',
    family: 'hcaptcha',
    url: 'https://accounts.hcaptcha.com/demo',
    sitekey: 'a5f74b19-9e45-40e0-b45d-47ff91b7a6c2',
    expect: ['unsupported', 'needs_human'],
    mayNeedPaid: false,
    ready: '!!document.querySelector(".h-captcha iframe")',
  },
  {
    id: '2captcha-recaptcha-demo',
    title: '2Captcha reCAPTCHA v2 demo',
    kind: 'demo',
    family: 'recaptcha_v2',
    url: 'https://2captcha.com/demo/recaptcha-v2',
    sitekey: '6LfD3PIbAAAAAJs_eEHvoOl75_83eXSqpPSRFJ_u',
    expect: ['solved'],
    mayNeedPaid: true,
    ready: '!!document.querySelector("#g-recaptcha iframe")',
    verdict: {
      submitSelector: 'main form button[type="submit"]',
      acceptedPattern: '"success"\\s*:\\s*true',
    },
  },
  {
    id: '2captcha-turnstile-demo',
    title: '2Captcha Turnstile demo',
    kind: 'demo',
    family: 'turnstile',
    url: 'https://2captcha.com/demo/cloudflare-turnstile',
    sitekey: '3x00000000000000000000FF',
    expect: ['solved'],
    mayNeedPaid: false,
    ready:
      '(document.querySelector("#cf-turnstile > div")?.getBoundingClientRect().height ?? 0) > 0',
    verdict: {
      submitSelector: 'main form button[type="submit"]',
      acceptedPattern: '"success"\\s*:\\s*true',
    },
    notes: 'Cloudflare public interactive test key.',
  },
]

export function tokenLengthExpression(family: CaptchaFamily): string {
  const fields = {
    recaptcha_v2: 'g-recaptcha-response',
    turnstile: 'cf-turnstile-response',
    hcaptcha: 'h-captcha-response',
    none: '',
  }
  return family === 'none'
    ? '0'
    : `(document.querySelector('[name="${fields[family]}"]')?.value?.length ?? 0)`
}
