import { expect, test } from 'bun:test'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import * as catalog from '../fixtures/catalog'
import { launchConfiguredBrowser } from './browser-launch'
import {
  DEFAULT_REAL_PAID_TASK_LIMIT,
  parseOptions,
  selectPages,
  serverEnv,
} from './run'
import { startStubMcp } from './stub-mcp'

const all = [
  ...catalog.FIXTURE_PAGES,
  ...catalog.DEMO_PAGES,
  ...catalog.SPOTTING_PAGES,
]

test('tests that spotting remains selectable within fixtures and all', () => {
  const spotting = selectPages(all, parseOptions(['--pages', 'spotting']))
  expect(spotting).toHaveLength(11)
  for (const group of ['fixtures', 'all'])
    expect(
      spotting.every((p) =>
        selectPages(all, parseOptions(['--pages', group])).includes(p),
      ),
    ).toBe(true)
  expect(selectPages(all, parseOptions(['--pages', 'fixtures']))).toHaveLength(
    19,
  )
  expect(selectPages(all, parseOptions(['--pages', 'demo']))).toHaveLength(8)
  expect(selectPages(all, parseOptions(['--pages', 'all']))).toHaveLength(27)
  expect(
    selectPages(
      all,
      parseOptions(['--pages', 'spotting', '--only', 'geetest-v4-frame']),
    ).map((p) => p.id),
  ).toEqual(['geetest-v4-frame'])
})

test('tests that the SDK is the default server and claw-server needs an endpoint', () => {
  expect(parseOptions([])).toMatchObject({
    server: 'sdk',
    pages: 'fixtures',
    keys: 'placeholder',
    paidTaskLimit: undefined,
    solveTimeout: 150_000,
    headed: true,
  })
  expect(
    parseOptions([
      '--server',
      'claw-server',
      '--mcp-url',
      'http://127.0.0.1:9402/mcp',
    ]).server,
  ).toBe('claw-server')
  expect(() => parseOptions(['--server', 'claw-server'])).toThrow(
    'launches only the Neo SDK',
  )
  expect(() => parseOptions(['--server', 'claw'])).toThrow('--server')
  expect(() => parseOptions(['--hosted-mode', '0'])).toThrow()
})

test('tests that real keys get a paid-task budget unless one is given', () => {
  expect(parseOptions(['--keys', 'real']).paidTaskLimit).toBe(
    DEFAULT_REAL_PAID_TASK_LIMIT,
  )
  expect(
    parseOptions(['--keys', 'real', '--paid-task-limit', '3']).paidTaskLimit,
  ).toBe(3)
  expect(parseOptions(['--paid-task-limit', '0']).paidTaskLimit).toBe(0)
  for (const args of [
    ['--paid-task-limit', '-1'],
    ['--paid-task-limit', '1.5'],
    ['--paid-task-limit', '2', '--mcp-url', 'http://127.0.0.1:9402/mcp'],
  ])
    expect(() => parseOptions(args)).toThrow('--paid-task-limit')
  expect(serverEnv({ BROWSEROS_CAPSOLVER_API_KEY_FILE: '/k' }, 4)).toEqual({
    BROWSEROS_CAPSOLVER_API_KEY_FILE: '/k',
    BROWSEROS_CAPTCHA_PAID_TASK_LIMIT: '4',
  })
  expect(serverEnv({}, undefined).BROWSEROS_CAPTCHA_PAID_TASK_LIMIT).toBe(
    undefined,
  )
})

test('tests that the runner accepts the cross-site frame host flag and repeated browser flags', () => {
  expect(
    parseOptions(['--fixture-frame-host', 'captcha-frame.test'])
      .fixtureFrameHost,
  ).toBe('captcha-frame.test')
  expect(
    parseOptions([
      '--',
      '--pages',
      'all',
      '--browser-arg',
      '--one',
      '--browser-arg=--host-resolver-rules=MAP fixture.test 127.0.0.1',
    ]).browserArgs,
  ).toEqual(['--one', '--host-resolver-rules=MAP fixture.test 127.0.0.1'])
})

test('tests that invalid CLI options are usage errors', () => {
  for (const args of [
    ['--pages', 'bad'],
    ['--keys', 'bad'],
    ['--solve-timeout', '0'],
    ['--solve-timeout', 'NaN'],
    ['--solve-timeout', '2147483648'],
    ['--solve-timeout', '900001'],
    ['--only', ''],
    ['--unknown'],
    ['--mcp-url', 'file:///tmp/mcp'],
  ]) {
    expect(() => parseOptions(args)).toThrow()
  }
  expect(() => selectPages([], parseOptions(['--only', 'missing']))).toThrow()
  expect(parseOptions(['--solve-timeout', '900000']).solveTimeout).toBe(900_000)
})

test('tests that dev watch accepts a startup budget and rejects conflicting launch flags', () => {
  expect(parseOptions(['--dev-watch'])).toMatchObject({
    devWatch: true,
    startupTimeout: 600_000,
  })
  expect(
    parseOptions(['--dev-watch', '--startup-timeout', '120000']),
  ).toMatchObject({ startupTimeout: 120_000 })
  for (const flags of [
    ['--mcp-url', 'http://127.0.0.1:9402/mcp'],
    ['--headless'],
    ['--browser-arg=--anything'],
    ['--startup-timeout', '0'],
    ['--startup-timeout', 'NaN'],
    ['--startup-timeout', '2147483648'],
  ])
    expect(() => parseOptions(['--dev-watch', ...flags])).toThrow()
})

test('tests that browser flags with spaces arrive as one argument and environment is restored', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'captcha-launch-test-'))
  const binary = join(directory, "fake browser's binary")
  await writeFile(binary, '#!/bin/sh\nprintf "%s\\n" "$@"\n', { mode: 0o700 })
  const previous = process.env.BROWSEROS_BINARY
  process.env.BROWSEROS_BINARY = binary
  let wrapper = ''
  try {
    await launchConfiguredBrowser(
      true,
      ['--host-resolver-rules=MAP fixture.test 127.0.0.1'],
      async () => {
        wrapper = process.env.BROWSEROS_BINARY ?? ''
        const result = Bun.spawnSync([wrapper, '--from-harness'])
        expect(result.exitCode).toBe(0)
        expect(result.stdout.toString().trim().split('\n')).toEqual([
          '--from-harness',
          '--host-resolver-rules=MAP fixture.test 127.0.0.1',
        ])
        expect(process.env.BROWSEROS_TEST_HEADLESS).toBe('false')
        return {
          cdpPort: 1,
          userDataDir: directory,
          isRunning: async () => true,
          kill: async () => {},
        }
      },
    )
    expect(parseOptions(['--headless']).headed).toBe(false)
    expect(parseOptions(['--headed']).headed).toBe(true)
    expect(process.env.BROWSEROS_BINARY).toBe(binary)
    expect(await Bun.file(wrapper).exists()).toBe(false)
  } finally {
    if (previous === undefined) delete process.env.BROWSEROS_BINARY
    else process.env.BROWSEROS_BINARY = previous
    await rm(directory, { recursive: true, force: true })
  }
})

async function cli(
  scenario: Parameters<typeof startStubMcp>[0]['scenario'],
  args: string[],
) {
  const out = await mkdtemp(join(tmpdir(), 'captcha-cli-test-'))
  const stub = await startStubMcp({ scenario, pages: catalog.FIXTURE_PAGES })
  try {
    const child = Bun.spawn(
      [
        process.execPath,
        join(import.meta.dir, 'run.ts'),
        '--mcp-url',
        `${stub.url}/mcp`,
        '--fixture-origin',
        'http://fixtures.invalid',
        '--only',
        'no-captcha',
        '--out',
        out,
        ...args,
      ],
      { stdout: 'pipe', stderr: 'pipe' },
    )
    const [stdout, stderr, code] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
      child.exited,
    ])
    return {
      code,
      stdout,
      stderr,
      results: await Bun.file(join(out, 'results.json')).json(),
      preflight: await Bun.file(join(out, 'preflight.json')).json(),
      markdown: await Bun.file(join(out, 'results.md')).text(),
    }
  } finally {
    await stub.stop()
    await rm(out, { recursive: true, force: true })
  }
}

test('tests that an SDK endpoint run writes the preflight, an array of results and exits zero', async () => {
  const run = await cli('contract', [])
  expect({ code: run.code, stderr: run.stderr }).toEqual({
    code: 0,
    stderr: '',
  })
  expect(run.preflight).toMatchObject({ ok: true, server: 'sdk' })
  expect(run.results).toHaveLength(1)
  expect(run.results[0]).toMatchObject({
    id: 'no-captcha',
    verdict: 'already_clear',
    outcome: 'PASS',
  })
  expect(run.markdown).toContain('Server: sdk at ')
  expect(run.markdown).toContain('(endpoint-owned)')
  expect(run.markdown).toContain('PASS=1 FAIL=0 UNVERIFIED=0')
})

test('tests that a keyless SDK endpoint stops the run as a setup blocker with exit two', async () => {
  const run = await cli('no-tool', [])
  expect(run.code).toBe(2)
  expect(run.stderr).toContain('SETUP BLOCKER: neo.solveCaptcha is not offered')
  expect(run.results).toEqual([])
  expect(run.preflight.blocker).toContain('no readable CAPTCHA provider key')
  const absent = await cli('no-tool', ['--expect-tool-missing'])
  expect(absent.code).toBe(0)
  expect(absent.results[0]).toMatchObject({
    error: 'host/captcha_unavailable',
    outcome: 'PASS',
  })
})
