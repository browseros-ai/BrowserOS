import { afterAll, beforeEach, describe, expect, it } from 'bun:test'
import { tmpdir } from 'node:os'

const requests: { url: string; body: unknown; headers: Headers }[] = []
let status = 200
let acknowledgement: string | number = 'Ok'
const endpoint = Bun.serve({
  hostname: '127.0.0.1',
  port: 0,
  fetch: async (request) => {
    requests.push({
      url: request.url,
      body: await request.json(),
      headers: request.headers,
    })
    return Response.json(
      { status: status === 200 ? acknowledgement : 0 },
      { status },
    )
  },
})

const originalKey = process.env.VITE_CLAW_POSTHOG_KEY
const originalHost = process.env.VITE_CLAW_POSTHOG_HOST
process.env.VITE_CLAW_POSTHOG_KEY = 'test-project-key'
process.env.VITE_CLAW_POSTHOG_HOST = endpoint.url.toString()
const { submitUltrafastSignup } = await import('./ultrafast-signup')

beforeEach(() => {
  requests.length = 0
  status = 200
  acknowledgement = 'Ok'
})

afterAll(() => {
  endpoint.stop(true)
  if (originalKey === undefined) delete process.env.VITE_CLAW_POSTHOG_KEY
  else process.env.VITE_CLAW_POSTHOG_KEY = originalKey
  if (originalHost === undefined) delete process.env.VITE_CLAW_POSTHOG_HOST
  else process.env.VITE_CLAW_POSTHOG_HOST = originalHost
})

const signup = {
  email: 'ada@example.com',
  price: 19 as const,
  distinctId: 'anonymous-installation-id',
}

describe('waitlist delivery', () => {
  it.each(['Ok', 1])(
    'tests that the PostHog acknowledgement %j confirms signup',
    async (value) => {
      acknowledgement = value
      await expect(submitUltrafastSignup(signup)).resolves.toBeUndefined()
    },
  )

  it('tests that signup reaches ingestion without starting the analytics SDK', async () => {
    await submitUltrafastSignup(signup)

    expect(requests).toHaveLength(1)
    expect(requests[0]?.url).toBe(`${endpoint.url}i/v0/e/`)
    expect(requests[0]?.headers.get('content-type')).toBe('application/json')
    expect(requests[0]?.headers.get('cookie')).toBeNull()
    expect(requests[0]?.headers.get('referer')).toBeNull()
    expect(requests[0]?.body).toEqual({
      api_key: 'test-project-key',
      event: 'ultrafast_waitlist_joined',
      distinct_id: signup.distinctId,
      properties: {
        email: signup.email,
        price_usd: signup.price,
        $process_person_profile: false,
      },
    })
  })

  it('tests that an ingestion failure is returned to the form', async () => {
    status = 503
    await expect(submitUltrafastSignup(signup)).rejects.toThrow(
      'Waitlist signup failed',
    )
  })

  it.each([undefined, '', '   '])(
    'tests that a missing project key (%j) cannot silently accept a signup',
    (key) => {
      const env = { ...process.env }
      if (key === undefined) delete env.VITE_CLAW_POSTHOG_KEY
      else env.VITE_CLAW_POSTHOG_KEY = key
      const result = Bun.spawnSync({
        cmd: [
          process.execPath,
          '--eval',
          `import { submitUltrafastSignup } from ${JSON.stringify(`${import.meta.dir}/ultrafast-signup.ts`)};
           globalThis.fetch = () => { throw new Error('unexpected network request') };
           try { await submitUltrafastSignup(${JSON.stringify(signup)}); console.log('accepted') }
           catch (error) { console.log(error.message) }`,
        ],
        cwd: tmpdir(),
        env,
      })
      expect(result.exitCode).toBe(0)
      expect(result.stdout.toString().trim()).toBe(
        'Waitlist signup is unavailable',
      )
    },
  )
})
