export type CaptchaFamily =
  | 'recaptcha_v2'
  | 'recaptcha_v3'
  | 'turnstile'
  | 'hcaptcha'
  | 'geetest'
  | 'aws_waf'
  | 'arkose'
  | 'perimeterx'
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
/**
 * The test bed's own classification of `neo.solveCaptcha`'s `reason` (or claw-server's `status`),
 * used for catalog expectations and runner rules. The Neo SDK itself returns no `verdict`.
 */
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
  /** The control the runner clicks after the solve. Absent: the page reports by itself. */
  submitSelector?: string
  acceptedPattern: string
  rejectedPattern?: string
}
/** Something the runner does to the page before the solve, as a visitor would. */
export interface PageTrigger {
  /** Clicked with the page's own `click()`. */
  click: string
  /** A page expression the runner then waits for; the solve runs either way. */
  until: string
}
/**
 * A value the runner records from the page after the solve and the site check. It never decides
 * the outcome: it keeps what a person would want to read off the page, such as a v3 score.
 */
export interface PageReading {
  label: string
  /** A page expression that is a string once the value is there, and null until then. */
  expression: string
  /** How long to wait for a string; the reading is null after that. Default: the probe timeout. */
  waitMs?: number
}
export interface CaptchaPage {
  id: string
  title: string
  kind: 'fixture' | 'demo'
  family: CaptchaFamily
  sitekey?: string
  path?: string
  url?: string
  /** Classified verdicts that pass. A success verdict on a CAPTCHA page also needs the site's acceptance. */
  expect: Verdict[]
  /** When set, a matching verdict also needs one of these reasons. */
  expectReason?: string[]
  /** When set, the SDK's `attempted` flag must match (ignored for claw-server, where it is `null`). */
  expectAttempted?: boolean
  /**
   * A type refusal (`unsolved` / `unsupported_type`, claw-server's `unsupported`) passes here
   * too. Elsewhere it fails, unless `expectReason` asks for it.
   */
  allowUnsupported?: boolean
  expectType?: CaptchaType
  maxPaidTasks?: number
  mayNeedPaid: boolean
  /**
   * Paid providers refuse this page's vendor test key; when the widget already holds its token
   * Neo asks one, gets `provider_error` with no paid task, and the page's own token still passes
   * the site.
   */
  providerRefusesKey?: boolean
  /**
   * Obvious fake token embedded by a stale-token fixture before the solve. The fixture server
   * rejects submissions carrying this exact value (`REJECTED: stale-token`), so a pass proves
   * Neo wrote a new token the vendor accepted.
   */
  staleToken?: string
  ready: string
  trigger?: PageTrigger
  site?: SiteCheck
  reading?: PageReading
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

/** Detected, and refused as a type Neo does not solve: never a solve or attempt. */
const unsupported = (type: CaptchaType) => ({
  expect: ['unsolved'] as Verdict[],
  expectType: type,
  expectReason: ['unsupported_type'],
  expectAttempted: false,
})

export const FIXTURE_PAGES: readonly CaptchaPage[] = [
  fixture(
    'recaptcha-v2-test',
    'reCAPTCHA v2 test checkbox',
    'recaptcha_v2',
    '/recaptcha-v2.html',
    '6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI',
  ),
  {
    ...fixture(
      'recaptcha-v2-stale',
      'reCAPTCHA v2 test checkbox with a stale token',
      'recaptcha_v2',
      '/recaptcha-v2-stale.html',
      '6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI',
    ),
    staleToken: 'STALE-TOKEN-recaptcha-v2-do-not-accept',
    expectReason: ['checkbox_passed'],
    expectAttempted: true,
    maxPaidTasks: 0,
    notes:
      'Writes a stale token into g-recaptcha-response before the solve; clicking the unchecked checkbox must replace it for free.',
  },
  {
    ...fixture(
      'recaptcha-v2-invisible-bind',
      'Invisible reCAPTCHA v2 bound to a button',
      'recaptcha_v2',
      '/recaptcha-v2-invisible-bind.html',
      '6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI',
      ['already_clear'],
    ),
    expectType: 'recaptcha_v2',
    expectAttempted: false,
    maxPaidTasks: 0,
  },
  {
    ...fixture(
      'turnstile-pass',
      'Turnstile always passes',
      'turnstile',
      '/turnstile-pass.html',
      '1x00000000000000000000AA',
      ['solved', 'token_applied'],
    ),
    expectAttempted: true,
    providerRefusesKey: true,
    notes:
      "Always-pass test key: neo.solveCaptcha usually lands before the widget's token and passes free (auto_passed). When the widget wins the race, paid providers refuse this test key (provider_error, 0 paid tasks) while the page's own token still passes the site.",
  },
  {
    ...fixture(
      'turnstile-stale',
      'Turnstile failed widget with a stale token',
      'turnstile',
      '/turnstile-stale.html',
      '3x00000000000000000000FF',
      ['token_applied', 'solved'],
    ),
    staleToken: 'STALE-TOKEN-turnstile-do-not-accept',
    expectAttempted: true,
    maxPaidTasks: 1,
    mayNeedPaid: true,
    notes:
      'Static failed-widget box with no vendor script and no checkbox to click, so Neo must mint a paid token; 2Captcha mints this test key.',
  },
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
  {
    ...fixture(
      'turnstile-stale-interactive',
      'Turnstile interactive challenge with a stale token',
      'turnstile',
      '/turnstile-stale-interactive.html',
      '3x00000000000000000000FF',
    ),
    staleToken: 'STALE-TOKEN-turnstile-interactive-do-not-accept',
    expectReason: ['checkbox_passed'],
    expectAttempted: true,
    maxPaidTasks: 0,
    notes:
      'Writes a stale token into cf-turnstile-response while the checkbox is unpassed; clicking the checkbox must replace it for free.',
  },
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
    // A static wall never clears: Neo watches it for its 10 s and hands it to a person.
    expect: ['unsolved'] as Verdict[],
    expectType: 'cloudflare_challenge',
    expectReason: ['challenge_not_passed'],
    expectAttempted: true,
    notes:
      'Detected as cloudflare_challenge by its challenge script; its Turnstile is the wall checkbox. It never clears, so Neo reports challenge_not_passed.',
  },
  {
    ...fixture(
      'no-captcha',
      'Ordinary form',
      'none',
      '/no-captcha.html',
      undefined,
      ['already_clear'],
    ),
    expectAttempted: false,
  },
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
  {
    ...fixture(
      'perimeterx-modal',
      'PerimeterX press-and-hold check over a page',
      'perimeterx',
      '/perimeterx-modal.html',
    ),
    // Its `type` is `unknown` on purpose: claw-server's closed schema does not list it yet.
    ...unsupported('unknown'),
    notes:
      'Detected by its frame over the page and its #px-captcha; the message and events name PerimeterX.',
  },
  {
    ...fixture(
      'perimeterx-waiting',
      'PerimeterX frame inserted, not shown',
      'none',
      '/perimeterx-waiting.html',
      undefined,
      ['already_clear'],
    ),
    expectAttempted: false,
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
    expect: ['solved', 'token_applied'],
    expectAttempted: true,
    mayNeedPaid: false,
    providerRefusesKey: true,
    ready:
      '(document.querySelector(".cf-turnstile > div")?.getBoundingClientRect().height ?? 0) > 0',
    site: {
      submitSelector: 'form[action="/handler"] button[type="submit"]',
      acceptedPattern: '"success"\\s*:\\s*true',
      rejectedPattern: '"success"\\s*:\\s*false',
    },
    // The handler prints siteverify's JSON; keep its verdict and error codes beside the outcome.
    reading: {
      label: 'siteverify',
      expression:
        '(document.body?.innerText.match(/"success"\\s*:\\s*(true|false)[\\s\\S]{0,160}/) ?? [])[0]?.replace(/\\s+/g, " ") ?? null',
    },
    notes:
      "Always-pass test key: neo.solveCaptcha usually lands before the widget's token and passes free (auto_passed). When the widget wins the race, paid providers refuse this test key (provider_error, 0 paid tasks) while the page's own token still passes the site.",
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
  ...[false, true].map(
    (triggered): CaptchaPage => ({
      id: `google-recaptcha-invisible-${triggered ? 'triggered' : 'untouched'}-demo`,
      title: `Google reCAPTCHA invisible demo, ${triggered ? 'solved after its submit shows the challenge' : 'solved before submit'}`,
      kind: 'demo',
      family: 'recaptcha_v2',
      url: 'https://www.google.com/recaptcha/api2/demo?invisible=true',
      sitekey: '6LfP0CITAAAAAHq9FOgCo7v_fb0-pmmH9VW3ziFs',
      expect: ['solved'],
      expectType: 'recaptcha_v2',
      mayNeedPaid: true,
      // reCAPTCHA enables the bound button once its widget is set up.
      ready:
        '!!document.querySelector("#recaptcha-demo iframe") && document.querySelector("#recaptcha-demo-submit")?.disabled === false',
      // The bound button runs grecaptcha.execute(); the image challenge appears in the bframe.
      trigger: triggered
        ? {
            click: '#recaptcha-demo-submit',
            until:
              '[...document.querySelectorAll("iframe[src*=\\"/recaptcha/api2/bframe\\"]")].some(f => getComputedStyle(f).visibility === "visible" && f.getBoundingClientRect().height > 100)',
          }
        : undefined,
      // Its callback submits the form itself; the runner clicks only if the form is still there.
      site: {
        submitSelector: '#recaptcha-demo-submit',
        acceptedPattern: 'Verification Success',
      },
      notes: triggered
        ? 'The owner order: the visitor submits first, so the challenge is up when Neo solves.'
        : 'The widget is untouched when Neo solves; the runner then submits.',
    }),
  ),
  {
    id: '2captcha-recaptcha-enterprise-demo',
    title: '2Captcha reCAPTCHA v2 Enterprise demo',
    kind: 'demo',
    family: 'recaptcha_v2',
    url: 'https://2captcha.com/demo/recaptcha-v2-enterprise',
    sitekey: '6Lf26sUnAAAAAIKLuWNYgRsFUfmI-3Lex3xT5N-s',
    expect: ['solved'],
    expectType: 'recaptcha_v2',
    mayNeedPaid: true,
    // The page renders its widget after hydration; a solve before that finds no CAPTCHA.
    ready:
      '(document.querySelector(".g-recaptcha iframe")?.getBoundingClientRect().height ?? 0) > 0',
    site: {
      submitSelector: 'main form button[type="submit"]',
      acceptedPattern: '"success"\\s*:\\s*true',
      rejectedPattern: '"success"\\s*:\\s*false',
    },
  },
  {
    id: 'recaptcha-v3-scores-demo',
    title: 'Google reCAPTCHA v3 request-scores demo',
    kind: 'demo',
    family: 'recaptcha_v3',
    url: 'https://recaptcha-demo.appspot.com/recaptcha-v3-request-scores.php',
    sitekey: '6LdKlZEpAAAAAAOQjzC2v_d36tWxCl6dWsozdSy9',
    expect: ['already_clear', 'solved'],
    expectType: 'recaptcha_v3',
    mayNeedPaid: false,
    ready: '!document.querySelector(".step1")?.classList.contains("hidden")',
    // The page sends its own v3 token to its server on load and prints the JSON reply.
    site: {
      acceptedPattern: '"success"\\s*:\\s*true',
      rejectedPattern: '"success"\\s*:\\s*false',
    },
    reading: {
      label: 'score',
      expression:
        '(document.querySelector(".step3:not(.hidden) .response")?.textContent.match(/"score"\\s*:\\s*([0-9.]+)/) ?? [])[1] ?? null',
    },
    notes: 'v3 never blocks: the score is what the site would judge.',
  },
  {
    id: 'scrapingcourse-cloudflare-challenge',
    title: 'Cloudflare full-page challenge (ScrapingCourse)',
    kind: 'demo',
    family: 'turnstile',
    url: 'https://www.scrapingcourse.com/cloudflare-challenge',
    expect: ['solved', 'already_clear'],
    expectType: 'cloudflare_challenge',
    // The cleared page reports by itself; there is nothing to submit.
    site: { acceptedPattern: 'You bypassed the Cloudflare challenge' },
    // A home IP clears by itself in 2-3 s; a datacenter IP shows the checkbox, which this
    // version of Neo does not click yet.
    mayNeedPaid: true,
    ready:
      '!!window._cf_chl_opt || !/Just a moment|Verify you are human/i.test(document.title + " " + (document.body?.innerText ?? ""))',
    reading: {
      label: 'left-challenge',
      expression:
        '/Just a moment|Verify you are human|security verification/i.test(document.title + " " + (document.body?.innerText ?? "")) ? null : document.title',
      waitMs: 30_000,
    },
    notes:
      'Passes when Cloudflare lets the browser through and the page says so. The reading is the page title once the challenge is gone.',
  },
]

/**
 * The owner's six-site check, in the owner's order (`--pages owner`). Site 2, invisible v2, runs
 * twice: untouched, and after its own submit has shown the image challenge.
 */
export const OWNER_PAGE_IDS: readonly string[] = [
  'google-recaptcha-demo',
  'google-recaptcha-invisible-untouched-demo',
  'google-recaptcha-invisible-triggered-demo',
  '2captcha-recaptcha-enterprise-demo',
  'cloudflare-turnstile-demo',
  'recaptcha-v3-scores-demo',
  'scrapingcourse-cloudflare-challenge',
]

export function tokenLengthExpression(family: CaptchaFamily): string {
  const fields: Record<CaptchaFamily, string> = {
    recaptcha_v2: 'g-recaptcha-response',
    recaptcha_v3: '',
    turnstile: 'cf-turnstile-response',
    hcaptcha: 'h-captcha-response',
    geetest: '',
    aws_waf: '',
    arkose: 'fc-token',
    perimeterx: '',
    none: '',
  }
  return fields[family]
    ? `(document.querySelector('[name="${fields[family]}"]')?.value?.length ?? 0)`
    : '0'
}
