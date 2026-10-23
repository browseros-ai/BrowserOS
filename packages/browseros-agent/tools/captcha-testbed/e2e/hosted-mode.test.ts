import { expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { FIXTURE_PAGES } from '../fixtures/catalog'
import { checkDevPreflight, renderPreflight, startDevWatch } from './dev-watch'
import * as runner from './run'
import { startStubMcp } from './stub-mcp'

test('tests that owned servers default to hosted mode and accept desktop mode', () => {
  expect(runner.parseOptions([]).hostedMode).toBe('1')
  expect(runner.parseOptions(['--dev-watch']).hostedMode).toBe('1')
  expect(runner.parseOptions(['--hosted-mode', '0']).hostedMode).toBe('0')
  expect(
    runner.parseOptions(['--dev-watch', '--hosted-mode=1']).hostedMode,
  ).toBe('1')
})

test('tests that invalid hosted modes are clear usage errors', () => {
  for (const value of ['', '2', 'true', '01'])
    expect(() => runner.parseOptions([`--hosted-mode=${value}`])).toThrow(
      '--hosted-mode must be 1 or 0',
    )
})

test('tests that an external MCP endpoint rejects either explicit hosted mode', () => {
  const endpoint = ['--mcp-url', 'http://127.0.0.1:9402/mcp']
  expect(runner.parseOptions(endpoint).mcpUrl).toBe(endpoint[1])
  for (const mode of ['1', '0'])
    for (const args of [
      [...endpoint, '--hosted-mode', mode],
      [`--hosted-mode=${mode}`, ...endpoint],
    ])
      expect(() => runner.parseOptions(args)).toThrow(
        '--hosted-mode cannot be combined with --mcp-url',
      )
})

for (const hostedMode of ['1', '0'] as const) {
  test(`tests that harness spawn receives hosted mode ${hostedMode} and key paths`, async () => {
    let env: NodeJS.ProcessEnv = {}
    const server = {
      baseUrl: 'http://127.0.0.1:9402',
      toolOutputDir: '/fake/output',
      homeDir: '/fake/home',
      stop: async () => {},
    }
    expect(
      await runner.startHarnessServer(
        9401,
        {
          hostedMode,
          keyEnv: {
            BROWSERCLAW_HOSTED_MODE: hostedMode === '1' ? '0' : '1',
            BROWSEROS_CAPSOLVER_API_KEY_FILE: '/fake/key',
          },
        },
        async (port, options) => {
          expect(port).toBe(9401)
          env = options?.env ?? {}
          return server
        },
      ),
    ).toBe(server)
    expect(env.BROWSERCLAW_HOSTED_MODE).toBe(hostedMode)
    expect(env.BROWSEROS_CAPSOLVER_API_KEY_FILE).toBe('/fake/key')
  })

  test(`tests that dev-watch spawn receives hosted mode ${hostedMode} over inherited key env`, async () => {
    const out = await mkdtemp(join(tmpdir(), 'captcha-hosted-test-'))
    let env: NodeJS.ProcessEnv = {}
    try {
      const watch = await startDevWatch(
        {
          out,
          startupTimeout: 100,
          ...(hostedMode === '0' ? { hostedMode } : {}),
          keyEnv: {
            BROWSERCLAW_HOSTED_MODE: hostedMode === '1' ? '0' : '1',
            BROWSEROS_CAPSOLVER_API_KEY_FILE: '/fake/key',
          },
        },
        {
          spawn: (childEnv) => {
            env = childEnv
            return { pid: undefined, exited: Promise.resolve() }
          },
          processes: async () => '',
        },
      )
      try {
        expect(env.BROWSERCLAW_HOSTED_MODE).toBe(hostedMode)
        expect(env.BROWSEROS_CAPSOLVER_API_KEY_FILE).toBe('/fake/key')
      } finally {
        await watch.stop()
      }
    } finally {
      await rm(out, { recursive: true, force: true })
    }
  })

  test(`tests that dev-watch preflight records hosted mode ${hostedMode}`, async () => {
    const preflight = await checkDevPreflight(
      'http://127.0.0.1:9402/mcp',
      hostedMode === '0',
      {
        health: async () => ({ status: 'ok', browser: { connected: true } }),
        tools: async () =>
          hostedMode === '1' ? [{ name: 'solve_captcha' }] : [],
      },
      hostedMode,
    )
    expect(preflight.ok).toBe(true)
    expect(preflight.hostedMode).toBe(hostedMode)
    expect(renderPreflight(preflight)).toContain(`"hostedMode":"${hostedMode}"`)
  })
}

test('tests that external endpoint reports leave hosted mode to the endpoint', async () => {
  const out = await mkdtemp(join(tmpdir(), 'captcha-external-mode-test-'))
  const stub = await startStubMcp({
    scenario: 'contract',
    pages: FIXTURE_PAGES,
  })
  try {
    const child = Bun.spawn(
      [
        process.execPath,
        join(import.meta.dir, 'run.ts'),
        '--mcp-url',
        stub.url,
        '--fixture-origin',
        'http://fixtures.invalid',
        '--only',
        'no-captcha',
        '--out',
        out,
      ],
      { stdout: 'ignore', stderr: 'pipe' },
    )
    const errors = await new Response(child.stderr).text()
    expect({ code: await child.exited, errors }).toEqual({
      code: 0,
      errors: '',
    })
    expect(await Bun.file(join(out, 'results.md')).text()).toContain(
      'Hosted mode: endpoint-owned',
    )
  } finally {
    await stub.stop()
    await rm(out, { recursive: true, force: true })
  }
})
