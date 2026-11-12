export type CaptchaFamily =
  | 'recaptcha_v2'
  | 'turnstile'
  | 'hcaptcha'
  | 'geetest'
  | 'aws_waf'
  | 'arkose'
  | 'none'
/** neo-captcha's `CaptchaKind`: the `type` that `neo.solveCaptcha` reports. */
export type CaptchaType =
  | 'recaptcha_v2'
  | 'recaptcha_v3'
  | 'turnstile'
  | 'cloudflare_challenge'
  | 'hcaptcha'
  | 'geetest'
  | 'aws_waf'
  | 'arkose'
  | 'unknown'
/** `neo.solveCaptcha`'s `verdict`, exactly as the SDK returns it. */
export type Verdict =
  | 'solved'
  | 'token_applied'
  | 'already_clear'
  | 'unsolved'
  | 'cancelled'
  | 'unknown'
export const SUCCESS_VERDICTS: readonly Verdict[] = [
  'solved',
  'token_applied',
  'already_clear',
]
/** How the runner asks the page's own server whether it accepted the submission. */
export interface SiteCheck {
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
  /** Verdicts that pass. A success verdict on a CAPTCHA page also needs the site's acceptance. */
  expect: Verdict[]
  /** When set, a matching verdict also needs one of these reasons. */
  expectReason?: string[]
  /**
   * A type refusal (`unsolved` / `unsupported_type`, claw-server's `unsupported`) passes here
   * too. Elsewhere it fails, unless `expectReason` asks for it.
   */
  allowUnsupported?: boolean
  expectType?: CaptchaType
  maxPaidTasks?: number
  mayNeedPaid: boolean
  ready: string
  site?: SiteCheck
  notes?: string
}

/**
 * What a passing page proves. `solve`: the page's own server accepted the solve. `deliver`: the
 * solver's word only, as no server checks this page (never counted as independently solved).
 * `detect`: the right type, with no solve claimed.
 */
export function pageCheck(page: CaptchaPage): 'solve' | 'deliver' | 'detect' {
  if (!page.expect.some((verdict) => SUCCESS_VERDICTS.includes(verdict)))
    return 'detect'
  return page.site ? 'solve' : 'deliver'
}

function spotting(
  id: string,
  title: string,
  family: CaptchaFamily & CaptchaType,
): CaptchaPage {
  return {
    id,
    title,
    family,
    kind: 'fixture',
    path: `/${id}.html`,
    expect: ['unsolved'],
    expectType: family,
    allowUnsupported: true,
    maxPaidTasks: 0,
    mayNeedPaid: false,
    ready: `(s => s.rendered === true && s.loadedScripts?.length === ${id === 'aws-waf-interstitial' ? 2 : 1} && !s.errors?.includes('script-load-error'))(JSON.parse(document.documentElement.dataset.captchaFixture || '{}'))`,
  }
}

export const SPOTTING_PAGES: readonly CaptchaPage[] = [
  spotting('geetest-v4', 'GeeTest v4', 'geetest'),
  spotting('geetest-v4-frame', 'GeeTest v4 in a cross-site frame', 'geetest'),
  spotting(
    'geetest-v4-shadow',
    'GeeTest v4 in a closed shadow root',
    'geetest',
  ),
  spotting('geetest-v3', 'GeeTest v3', 'geetest'),
  spotting('aws-waf', 'AWS WAF JS API', 'aws_waf'),
  spotting('aws-waf-frame', 'AWS WAF in a cross-site frame', 'aws_waf'),
  spotting('aws-waf-shadow', 'AWS WAF in a closed shadow root', 'aws_waf'),
  spotting('aws-waf-interstitial', 'AWS WAF interstitial', 'aws_waf'),
  spotting('funcaptcha', 'FunCaptcha', 'arkose'),
  spotting('funcaptcha-frame', 'FunCaptcha in a cross-site frame', 'arkose'),
  spotting('funcaptcha-shadow', 'FunCaptcha in a closed shadow root', 'arkose'),
]

function fixture(
  id: string,
  title: string,
  family: CaptchaFamily,
  path: string,
  sitekey?: string,
  expect: Verdict[] = ['solved'],
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
    site: {
      submitSelector: '#submit',
      acceptedPattern: '^ACCEPTED',
      rejectedPattern: '^REJECTED',
    },
  }
}

/** Detected, and refused as a type Neo does not solve: never a solve. */
const unsupported = (type: CaptchaType) => ({
  expect: ['unsolved'] as Verdict[],
  expectType: type,
  expectReason: ['unsupported_type'],
})

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
    ['unsolved', 'unknown'],
  ),
  fixture(
    'turnstile-interactive',
    'Turnstile interactive challenge',
    'turnstile',
    '/turnstile-interactive.html',
    '3x00000000000000000000FF',
  ),
  fixture(
    'turnstile-cf-marker',
    'Turnstile beside Cloudflare bot management',
    'turnstile',
    '/turnstile-cf-marker.html',
    '3x00000000000000000000FF',
  ),
  {
    ...fixture(
      'cloudflare-wall',
      'Cloudflare challenge page',
      'turnstile',
      '/cloudflare-wall.html',
      '3x00000000000000000000FF',
    ),
    ...unsupported('cloudflare_challenge'),
    notes:
      'Detected as cloudflare_challenge by its challenge script; its Turnstile is the wall checkbox.',
  },
  fixture(
    'no-captcha',
    'Ordinary form',
    'none',
    '/no-captcha.html',
    undefined,
    ['already_clear'],
  ),
  {
    ...fixture(
      'hcaptcha-test',
      'hCaptcha test checkbox',
      'hcaptcha',
      '/hcaptcha.html',
      '10000000-ffff-ffff-ffff-000000000001',
    ),
    ...unsupported('hcaptcha'),
  },
]

export const DEMO_PAGES: readonly CaptchaPage[] = [
  ...['geetest', 'geetest-v4'].map(
    (slug): CaptchaPage => ({
      id: `2captcha-${slug}-demo`,
      title: `2Captcha ${slug === 'geetest' ? 'GeeTest v3' : 'GeeTest v4'} demo`,
      kind: 'demo',
      family: 'geetest',
      url: `https://2captcha.com/demo/${slug}`,
      expect: ['solved'],
      expectType: 'geetest',
      mayNeedPaid: true,
      ready:
        '(document.querySelector(".geetest_holder")?.getBoundingClientRect().height ?? 0) > 0',
      site: {
        submitSelector: 'main form button[type="submit"]',
        acceptedPattern:
          slug === 'geetest-v4'
            ? '"result"\\s*:\\s*"success"'
            : '"success"\\s*:\\s*true',
      },
    }),
  ),
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
    site: {
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
    site: {
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
    ...unsupported('hcaptcha'),
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
    site: {
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
    site: {
      submitSelector: 'main form button[type="submit"]',
      acceptedPattern: '"success"\\s*:\\s*true',
    },
    notes: 'Cloudflare public interactive test key.',
  },
  {
    id: 'arkose-demo',
    title: 'Arkose non-production demo',
    kind: 'demo',
    family: 'arkose',
    url: 'https://demo.arkoselabs.com/?key=11111111-1111-1111-1111-111111111111',
    sitekey: '11111111-1111-1111-1111-111111111111',
    expect: ['token_applied'],
    expectType: 'arkose',
    maxPaidTasks: 1,
    mayNeedPaid: true,
    ready:
      '(() => { const name = Object.keys(window).find(k => /^arkoseLabsClientApi/.test(k)); const state = name && window[name]?.state; return !!state?.iframe?.isConnected && !!state.token && !state.error && state.iframe.getBoundingClientRect().height > 0; })()',
    notes: 'Arkose-owned test key. SDK delivery only; no server verification.',
  },
]

export function tokenLengthExpression(family: CaptchaFamily): string {
  const fields = {
    recaptcha_v2: 'g-recaptcha-response',
    turnstile: 'cf-turnstile-response',
    hcaptcha: 'h-captcha-response',
    geetest: '',
    aws_waf: '',
    arkose: 'fc-token',
    none: '',
  }
  return fields[family]
    ? `(document.querySelector('[name="${fields[family]}"]')?.value?.length ?? 0)`
    : '0'
}
