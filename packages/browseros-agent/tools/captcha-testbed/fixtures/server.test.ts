import { afterAll, beforeAll, expect, test } from 'bun:test'
import { FIXTURE_PAGES } from './catalog'
import { type CaptchaFixtureServer, startCaptchaFixtureServer } from './server'

const cases = [
  [
    'recaptcha-v2-test',
    'recaptcha_v2',
    'g-recaptcha-response',
    '6LeIxAcTAAAAAGG-vFI1TnRWxMZNFuojJ4WifJWe',
  ],
  [
    'turnstile-pass',
    'turnstile',
    'cf-turnstile-response',
    '1x0000000000000000000000000000000AA',
  ],
  [
    'turnstile-block',
    'turnstile',
    'cf-turnstile-response',
    '2x0000000000000000000000000000000AA',
  ],
  [
    'turnstile-interactive',
    'turnstile',
    'cf-turnstile-response',
    '1x0000000000000000000000000000000AA',
  ],
  [
    'hcaptcha-test',
    'hcaptcha',
    'h-captcha-response',
    '0x0000000000000000000000000000000000000000',
  ],
] as const
const requests: { path: string; body: URLSearchParams; method: string }[] = []
const servers: CaptchaFixtureServer[] = []
const verifier = Bun.serve({
  hostname: '127.0.0.1',
  port: 0,
  async fetch(request) {
    const body = new URLSearchParams(await request.text())
    requests.push({
      path: new URL(request.url).pathname,
      body,
      method: request.method,
    })
    switch (body.get('response')) {
      case 'malformed':
        return new Response('{')
      case 'unavailable':
        return Response.json({ success: true }, { status: 503 })
      case 'truthy':
        return Response.json({ success: 'true' })
      case 'bad':
        return Response.json({
          success: false,
          'error-codes': ['invalid-input-response', '<script>'],
        })
      default:
        return Response.json({ success: true })
    }
  },
})
let fixture: CaptchaFixtureServer
beforeAll(async () => {
  fixture = await startCaptchaFixtureServer({
    verifyUrls: Object.fromEntries(
      ['recaptcha_v2', 'turnstile', 'hcaptcha'].map((family) => [
        family,
        `${verifier.url}${family}`,
      ]),
    ),
  })
  servers.push(fixture)
})
afterAll(async () => {
  await Promise.all(servers.map((server) => server.stop()))
  await verifier.stop(true)
})
const submit = (id: string, field = '', token = '') =>
  fetch(fixture.url(`/submit/${id}`), {
    method: 'POST',
    body: new URLSearchParams({ [field]: token }),
  })

test('tests that the server serves every catalog page and health check', async () => {
  expect(Number(new URL(fixture.origin).port)).toBeWithin(10101, 20203)
  expect(await (await fetch(fixture.url('/healthz'))).text()).toBe('ok')
  for (const page of FIXTURE_PAGES) {
    const response = await fetch(fixture.url(page.path ?? '/missing'))
    expect(response.status).toBe(200)
    expect(response.headers.get('content-type')).toContain('text/html')
    expect(await response.text()).toContain(`action="/submit/${page.id}"`)
  }
})

test.each(cases)(
  'tests that %s verifies its response field with the correct secret',
  async (id, family, field, secret) => {
    const response = await submit(id, field, 'token & value=123')
    expect(await response.text()).toContain('>ACCEPTED<')
    expect(requests.at(-1)).toEqual({
      path: `/${family}`,
      method: 'POST',
      body: new URLSearchParams({ secret, response: 'token & value=123' }),
    })
  },
)

test('tests that no-captcha always accepts without calling a verifier', async () => {
  const count = requests.length
  const response = await submit('no-captcha')
  expect(await response.text()).toContain('data-captcha-verdict="accepted"')
  expect(requests).toHaveLength(count)
})

test.each(cases)(
  'tests that %s rejects a missing or wrong response field',
  async (id) => {
    const count = requests.length
    expect(await (await submit(id, 'wrong-field', 'token')).text()).toContain(
      'REJECTED: missing-input-response',
    )
    expect(requests).toHaveLength(count)
  },
)

test('tests that rejected verification renders escaped vendor error codes', async () => {
  const response = await submit(
    'turnstile-pass',
    'cf-turnstile-response',
    'bad',
  )
  const html = await response.text()
  expect(html).toContain('data-captcha-verdict="rejected"')
  expect(html).toContain('REJECTED: invalid-input-response, &lt;script&gt;')
  expect(html).not.toContain('<script>')
})

test.each(['malformed', 'unavailable', 'truthy'])(
  'tests that invalid upstream verification fails closed (%s)',
  async (token) => {
    expect(
      await (
        await submit('turnstile-pass', 'cf-turnstile-response', token)
      ).text(),
    ).toContain('REJECTED:')
  },
)

test('tests that connection failures produce a rejected verdict', async () => {
  const stopped = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: () => new Response(),
  })
  const url = stopped.url.href
  await stopped.stop(true)
  const server = await startCaptchaFixtureServer({
    verifyUrls: { turnstile: url },
  })
  servers.push(server)
  const response = await fetch(server.url('/submit/turnstile-pass'), {
    method: 'POST',
    body: new URLSearchParams({ 'cf-turnstile-response': 'x' }),
  })
  expect(await response.text()).toContain('REJECTED: verification-unavailable')
})

test('tests that unrecognized paths, ids and methods are not served', async () => {
  for (const path of [
    '/unknown.html',
    '/server.ts',
    '/submit/unknown',
    '/submit/turnstile-pass',
  ]) {
    expect((await fetch(fixture.url(path))).status).toBe(404)
  }
  expect((await submit('unknown')).status).toBe(404)
  expect(
    (await fetch(fixture.url('/no-captcha.html'), { method: 'POST' })).status,
  ).toBe(404)
})

test('tests that publicHost advertises a hostname independently of the bind address', async () => {
  const server = await startCaptchaFixtureServer({
    hostname: '0.0.0.0',
    publicHost: 'host.docker.internal',
    port: 0,
  })
  servers.push(server)
  expect(new URL(server.origin).hostname).toBe('host.docker.internal')
  expect(server.url('no-captcha.html')).toBe(`${server.origin}/no-captcha.html`)
  const port = new URL(server.origin).port
  expect(await (await fetch(`http://127.0.0.1:${port}/healthz`)).text()).toBe(
    'ok',
  )
})

test('tests that an explicitly requested occupied port is not silently changed', async () => {
  await expect(
    startCaptchaFixtureServer({ port: Number(new URL(fixture.origin).port) }),
  ).rejects.toThrow()
})
