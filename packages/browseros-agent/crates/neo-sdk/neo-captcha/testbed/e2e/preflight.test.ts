import { expect, test } from 'bun:test'
import { checkPreflight, OWNER_PORTS, renderPreflight } from './preflight'
import { startStubMcp } from './stub-mcp'

const url = 'http://127.0.0.1:9402/mcp'
const health = { status: 'ok', browser: { connected: true } }
const sdk = (listed: boolean) => [
  {
    name: 'run',
    description: `Runs Playwright JavaScript.\n\nSDK methods:${listed ? '\n- neo.solveCaptcha(page) → { verdict }' : ''}`,
  },
]
const claw = (listed: boolean) =>
  ['tabs', 'evaluate_in_page', ...(listed ? ['solve_captcha'] : [])].map(
    (name) => ({ name }),
  )
const deps = (tools: { name: string; description?: string }[], h = health) => ({
  health: async () => h,
  tools: async () => tools,
})

test('tests that an SDK endpoint passes when it offers neo.solveCaptcha', async () => {
  const result = await checkPreflight(url, { server: 'sdk' }, deps(sdk(true)))
  expect(result).toMatchObject({
    server: 'sdk',
    tools: ['run'],
    solverListed: true,
    health,
    ok: true,
  })
  expect(result.blocker).toBeUndefined()
})

test('tests that a keyless SDK is a setup blocker, not a solver result', async () => {
  const result = await checkPreflight(url, { server: 'sdk' }, deps(sdk(false)))
  expect(result.ok).toBe(false)
  expect(result.solverListed).toBe(false)
  expect(result.blocker).toContain('no readable CAPTCHA provider key')
  expect(renderPreflight(result)).toContain('SETUP BLOCKER:')
  const absent = await checkPreflight(
    url,
    { server: 'sdk', expectToolMissing: true },
    deps(sdk(false)),
  )
  expect(absent.ok).toBe(true)
})

test('tests that a listed solver fails an absence run as an expectation, not a blocker', async () => {
  const result = await checkPreflight(
    url,
    { server: 'sdk', expectToolMissing: true },
    deps(sdk(true)),
  )
  expect(result.ok).toBe(false)
  expect(result.blocker).toBeUndefined()
  expect(result.error).toContain('neo.solveCaptcha is offered')
  expect(renderPreflight(result)).toContain('PREFLIGHT FAILED:')
})

test('tests that the SDK runner refuses claw-server and the reverse', async () => {
  const wrongServer = await checkPreflight(
    url,
    { server: 'sdk' },
    deps(claw(true)),
  )
  expect(wrongServer.blocker).toContain('the endpoint is claw-server')
  expect(wrongServer.blocker).toContain('--server claw-server')
  const wrongSdk = await checkPreflight(
    url,
    { server: 'claw-server' },
    deps(sdk(true)),
  )
  expect(wrongSdk.blocker).toContain('the endpoint is the Neo SDK')
  const keyless = await checkPreflight(
    url,
    { server: 'claw-server' },
    deps(claw(false)),
  )
  expect(keyless.blocker).toContain('solve_captcha is not offered')
  expect(
    (await checkPreflight(url, { server: 'claw-server' }, deps(claw(true)))).ok,
  ).toBe(true)
})

test('tests that a disconnected browser or a failing endpoint blocks before any page', async () => {
  for (const h of [
    { status: 'ok', browser: { connected: false } },
    { status: 'error', browser: { connected: true } },
  ])
    expect(
      (await checkPreflight(url, { server: 'sdk' }, deps(sdk(true), h)))
        .blocker,
    ).toContain('connected')
  const failing = await checkPreflight(
    url,
    { server: 'sdk' },
    {
      health: async () => {
        throw new Error('ECONNREFUSED')
      },
      tools: async () => [],
    },
  )
  expect(failing.blocker).toBe('preflight request failed: ECONNREFUSED')
})

test('tests that owner ports are refused before any request when asked', async () => {
  for (const port of OWNER_PORTS) {
    const result = await checkPreflight(
      `http://127.0.0.1:${port}/mcp`,
      { server: 'sdk', refuseOwnerPorts: true },
      {
        health: async () => {
          throw new Error('must not request owner health')
        },
        tools: async () => {
          throw new Error('must not request owner tools')
        },
      },
    )
    expect(result.blocker).toBe(`refusing owner port ${port}`)
  }
})

test('tests that preflight reads a live endpoint over HTTP and MCP', async () => {
  const stub = await startStubMcp({ scenario: 'contract', pages: [] })
  try {
    expect(
      await checkPreflight(`${stub.url}/mcp`, { server: 'sdk' }),
    ).toMatchObject({ ok: true, tools: ['run'], solverListed: true })
  } finally {
    await stub.stop()
  }
})
