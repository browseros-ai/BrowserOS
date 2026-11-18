/** Exercises the real process boundary without launching Chromium or compiling the SDK. */
import { afterEach, expect, test } from 'bun:test'
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  writeFile,
} from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import {
  computeAffectedSuites,
  SUITES,
} from '../../../../../ci/affected-suites'
import { browserRecord } from './browser-adapter'
import { strictFailures } from './cases/requirements'
import { cases } from './inventory'
import { jsonObject, numberField, stringField } from './json'
import type { BrowserRecord, CaseSpec } from './model'
import {
  parseOptions,
  partitionCases,
  resolveBrowser,
  verifyCoverage,
} from './run'
import { serverEnvironment, startServer } from './server'

const roots: string[] = []

afterEach(async () => {
  for (const root of roots.splice(0))
    await rm(root, { recursive: true, force: true })
})

async function scratch(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), 'sdk-desktop-process-'))
  roots.push(root)
  return root
}

async function executable(path: string, source: string): Promise<string> {
  await writeFile(path, `#!${process.execPath}\n${source}`, { mode: 0o755 })
  return path
}

function browser(root: string): BrowserRecord {
  const profile = join(root, 'profile')
  return {
    profile,
    home: join(root, 'browser-home'),
    downloads: join(root, 'downloads'),
    args: [`--user-data-dir=${profile}`],
    scale: 1,
  }
}

function receipt(
  selected: readonly CaseSpec[],
): Parameters<typeof verifyCoverage>[1] {
  return {
    partition: 'ordinary',
    startedAt: Date.now(),
    startupMs: 1,
    casesMs: 1,
    cleanupMs: 1,
    finishedAt: Date.now(),
    complete: true,
    expectedIds: selected.map((entry) => entry.id),
    results: selected.map((entry) => ({
      id: entry.id,
      status: entry.expectation.kind,
    })),
  }
}

test('CI runs the offline guards and strict typecheck before the source-built browser suite', () => {
  expect(SUITES['sdk-desktop-fixtures']).toEqual({
    suite: 'sdk-desktop-fixtures',
    command:
      'bun crates/neo-sdk/neo-server/tests/desktop-fixtures/check.ts --run',
    junit_path: 'test-results/sdk-desktop-fixtures.xml',
    needs_browser: true,
    needs_rust: true,
  })
  const paths = [
    'contracts/claw-mcp/fixtures/pages/form.html',
    'contracts/claw-mcp/tests/cases-act.ts',
    'crates/neo-sdk/neo-playwright-js/src/index.ts',
    'crates/neo-sdk/neo-server/tests/desktop-fixtures/check.ts',
  ]
  for (const path of paths) {
    const selected = computeAffectedSuites(
      [],
      [`packages/browseros-agent/${path}`],
    )
    expect(
      selected.some((entry) => entry.suite === 'sdk-desktop-fixtures'),
    ).toBe(true)
  }
  const selected = computeAffectedSuites(
    [
      {
        name: '@browseros/neo-playwright-js',
        path: 'crates/neo-sdk/neo-playwright-js',
      },
      { name: '@browseros/neo-server', path: 'crates/neo-sdk/neo-server' },
    ],
    paths.map((path) => `packages/browseros-agent/${path}`),
  )
  expect(
    selected.filter((entry) => entry.suite === 'sdk-desktop-fixtures'),
  ).toHaveLength(1)
  expect(selected.map((entry) => entry.suite)).toEqual(
    Object.keys(SUITES).filter((name) =>
      selected.some((entry) => entry.suite === name),
    ),
  )
  const full = computeAffectedSuites(
    [],
    ['packages/browseros-agent/ci/affected-suites.ts'],
  )
  expect(full).toEqual(Object.values(SUITES))
})

test('native density and terminal observation execute in dedicated fresh browsers', () => {
  const ordinary = partitionCases(cases, 'ordinary')
  const density = partitionCases(cases, 'density')
  const terminal = partitionCases(cases, 'terminal-select')
  expect(ordinary).toHaveLength(253)
  expect(density.map((entry) => entry.id)).toEqual([
    'screenshot-scale-001',
    'screenshot-scale-002',
  ])
  expect(terminal.map((entry) => entry.id)).toEqual(['act-017'])
  expect(
    new Set([...ordinary, ...density, ...terminal].map((entry) => entry.id))
      .size,
  ).toBe(256)
  expect(() => verifyCoverage(ordinary, receipt(ordinary))).not.toThrow()
})

test('aggregate coverage rejects duplicated aliases, unexecuted rows and XPASS', () => {
  const selected = cases.filter((entry) => entry.expectation.kind === 'xfail')
  expect(selected).toHaveLength(strictFailures.size)
  const duplicate = receipt(selected)
  const first = duplicate.results[0]
  if (!first) throw new Error('Expected strict failure cases')
  duplicate.results.push(first)
  expect(() => verifyCoverage(selected, duplicate)).toThrow(
    'every selected case',
  )
  const missing = receipt(selected)
  missing.results.pop()
  expect(() => verifyCoverage(selected, missing)).toThrow('every selected case')
  const xpass = receipt(selected)
  xpass.results = xpass.results.map((entry) => ({ ...entry, status: 'pass' }))
  expect(() => verifyCoverage(selected, xpass)).toThrow('every selected case')
})

test('CLI keeps explicit case and output values and rejects unknown options', () => {
  expect(
    parseOptions(['--case', 'act-001', '--out', '/tmp/sdk-result', '--check']),
  ).toEqual({ caseId: 'act-001', out: '/tmp/sdk-result', check: true })
  expect(() => parseOptions(['--case'])).toThrow('requires a value')
  expect(() => parseOptions(['--prebuilt'])).toThrow('Unknown option')
})

test('browser resolution honors CI binary before an existing launcher override', async () => {
  const root = await scratch()
  const ci = await executable(join(root, 'ci-browser'), '')
  const developer = await executable(join(root, 'developer-browser'), '')
  expect(
    await resolveBrowser({ BROWSEROS_BINARY: ci, NEO_E2E_BROWSER: developer }),
  ).toBe(await realpath(ci))
})

test('the executable adapter preserves PID, all launcher flags, CI extras and prelaunch downloads', async () => {
  const root = await scratch()
  const paths = browser(root)
  await mkdir(paths.home)
  const actual = await executable(
    join(root, 'actual-browser'),
    `
import { readFileSync } from 'node:fs'
console.log(JSON.stringify({
  pid: process.pid,
  args: process.argv.slice(2),
  home: process.env.HOME,
  preferences: JSON.parse(readFileSync(process.env.SDK_DESKTOP_FIXTURE_PROFILE_PREFERENCES, 'utf8')),
}))
`,
  )
  const recordPath = join(root, 'browser.json')
  const args = [
    `--user-data-dir=${paths.profile}`,
    '--remote-debugging-port=0',
    '--headless=new',
    'about:blank',
  ]
  const child = Bun.spawn({
    cmd: [join(import.meta.dir, 'browser-adapter.ts'), ...args],
    env: {
      ...process.env,
      HOME: paths.home,
      SDK_DESKTOP_FIXTURE_BROWSER: actual,
      SDK_DESKTOP_FIXTURE_BROWSER_RECORD: recordPath,
      SDK_DESKTOP_FIXTURE_DOWNLOADS: paths.downloads,
      SDK_DESKTOP_FIXTURE_SCALE: '2',
      SDK_DESKTOP_FIXTURE_PROFILE_PREFERENCES: join(
        paths.profile,
        'Default/Preferences',
      ),
      BROWSEROS_TEST_EXTRA_ARGS: '--no-sandbox --disable-dev-shm-usage',
    },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const [code, stdout, stderr] = await Promise.all([
    child.exited,
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ])
  expect(stderr).toBe('')
  expect(code).toBe(0)
  const output = jsonObject(JSON.parse(stdout), 'browser subprocess')
  expect(numberField(output, 'pid')).toBe(child.pid)
  const expectedArgs = [
    ...args,
    '--no-sandbox',
    '--disable-dev-shm-usage',
    '--force-device-scale-factor=2',
  ]
  expect(output.args).toEqual(expectedArgs)
  expect(output.home).toBe(paths.home)
  expect(output.preferences).toEqual({
    download: {
      default_directory: paths.downloads,
      prompt_for_download: false,
    },
  })
  expect(JSON.parse(await readFile(recordPath, 'utf8'))).toEqual({
    ...paths,
    args: expectedArgs,
    scale: 2,
  })
})

test('ordinary density stays native and extras cannot replace launcher ownership', () => {
  const env = {
    HOME: '/tmp/browser-home',
    SDK_DESKTOP_FIXTURE_DOWNLOADS: '/tmp/downloads',
    SDK_DESKTOP_FIXTURE_SCALE: '1',
  }
  const record = browserRecord(
    ['--user-data-dir=/tmp/profile', '--remote-debugging-port=0'],
    env,
  )
  expect(record.scale).toBe(1)
  expect(record.args).not.toContain('--force-device-scale-factor=2')
  expect(() =>
    browserRecord(record.args, {
      ...env,
      BROWSEROS_TEST_EXTRA_ARGS: '--user-data-dir=/tmp/other',
    }),
  ).toThrow('cannot replace launcher ownership')
})

test('sidecar environment admits no provider credentials or inherited host state', () => {
  const record = browser('/tmp/process-environment')
  const env = serverEnvironment(
    {
      root: '/tmp/owned-sdk',
      tokenFile: '/tmp/owned-sdk/token',
      socketPath: '/tmp/owned-sdk/host.sock',
    },
    record,
    {
      PATH: '/test/path',
      LANG: 'C',
      OPENAI_API_KEY: 'not-a-real-key',
      BROWSEROS_CAPSOLVER_API_KEY_FILE: '/unowned/key',
      CLAW_HOST_TCP: '127.0.0.1:1',
      CLAW_HOST_SOCKET: '/unowned/socket',
      BROWSERCLAW_DIR: '/unowned/state',
      HOME: '/unowned/home',
      LD_PRELOAD: '/unowned/library',
    },
  )
  expect(env.PATH).toBe('/test/path')
  expect(env.HOME).toBe('/tmp/owned-sdk/home')
  expect(env.CLAW_PROFILE_DIR).toBe(record.profile)
  expect(env.CLAW_DOWNLOADS_DIR).toBe(record.downloads)
  expect(env.CLAW_HOST_SOCKET).toBe('/tmp/owned-sdk/host.sock')
  expect(env).not.toHaveProperty('OPENAI_API_KEY')
  expect(env).not.toHaveProperty('BROWSEROS_CAPSOLVER_API_KEY_FILE')
  expect(env).not.toHaveProperty('CLAW_HOST_TCP')
  expect(env).not.toHaveProperty('LD_PRELOAD')
})

async function fakeServer(root: string, shutdownCode = 0): Promise<string> {
  return executable(
    join(root, 'sdk-server'),
    `
import { readFileSync } from 'node:fs'
import { createServer } from 'node:http'
const config = JSON.parse(readFileSync(process.argv[3], 'utf8'))
const token = readFileSync(process.env.CLAW_HOST_TOKEN_FILE, 'utf8').trim()
const agent = createServer((request, response) => {
  response.writeHead(request.url === '/system/ready' ? 200 : 404).end('{}')
})
const host = createServer((request, response) => {
  const valid = request.url === '/v1/settings' && request.headers.authorization === 'Bearer ' + token
  console.log(JSON.stringify({ authenticatedSettings: valid }))
  response.writeHead(valid ? 200 : 401).end(JSON.stringify({ settings: {} }))
})
await new Promise((resolve) => agent.listen(config.ports.server, '127.0.0.1', resolve))
console.log(JSON.stringify({ config, home: process.env.HOME, profile: process.env.CLAW_PROFILE_DIR, downloads: process.env.CLAW_DOWNLOADS_DIR }))
setTimeout(() => host.listen(process.env.CLAW_HOST_SOCKET), 150)
process.on('SIGTERM', () => {
  console.log('received SIGTERM')
  agent.close()
  host.close(() => process.exit(${shutdownCode}))
})
`,
  )
}

test('sidecar waits for authenticated host readiness and stops only its own state', async () => {
  const root = await scratch()
  const record = browser(root)
  for (const path of [record.profile, record.home, record.downloads])
    await mkdir(path)
  const logPath = join(root, 'server.log')
  const server = await startServer({
    cdpPort: 43210,
    binary: await fakeServer(root),
    browser: record,
    logPath,
  })
  try {
    expect(Number(new URL(server.baseUrl).port)).not.toBe(43210)
    const output = await readFile(logPath, 'utf8')
    expect(output).toContain('"authenticatedSettings":true')
    const first = output.split('\n')[0]
    const facts = jsonObject(JSON.parse(first), 'server facts')
    expect(facts.profile).toBe(record.profile)
    expect(facts.downloads).toBe(record.downloads)
    expect(stringField(facts, 'home')).not.toBe(record.home)
    const config = jsonObject(facts.config, 'launch config')
    expect(jsonObject(config.ports, 'server ports').cdp).toBe(43210)
  } finally {
    await server.stop()
  }
  await server.stop()
  expect(await readFile(logPath, 'utf8')).toContain('received SIGTERM')
  await expect(access(dirname(server.socketPath))).rejects.toThrow()
  for (const path of [record.profile, record.home, record.downloads])
    await access(path)
  expect(await Bun.file(`${logPath}.lifecycle.json`).json()).toMatchObject({
    cleaned: true,
    forced: false,
    exitCode: 0,
  })
})

test('a nonzero sidecar shutdown is a cleanup failure even after private state is removed', async () => {
  const root = await scratch()
  const record = browser(root)
  await mkdir(record.profile)
  const logPath = join(root, 'server.log')
  const server = await startServer({
    cdpPort: 43211,
    binary: await fakeServer(root, 7),
    browser: record,
    logPath,
  })
  await expect(server.stop()).rejects.toThrow('shutdown exited 7')
  await expect(access(dirname(server.socketPath))).rejects.toThrow()
  expect(await Bun.file(`${logPath}.lifecycle.json`).json()).toMatchObject({
    cleaned: true,
    exitCode: 7,
  })
})

test('an SDK process which exits during startup cannot attach to another listener', async () => {
  const root = await scratch()
  const binary = await executable(
    join(root, 'sdk-exits'),
    "console.error('intentional startup failure'); process.exit(2)",
  )
  const logPath = join(root, 'server.log')
  await expect(
    startServer({ cdpPort: 43212, binary, browser: browser(root), logPath }),
  ).rejects.toThrow('startup')
  expect(await readFile(logPath, 'utf8')).toContain(
    'intentional startup failure',
  )
  expect(await Bun.file(`${logPath}.lifecycle.json`).json()).toMatchObject({
    cleaned: true,
    exitCode: 2,
  })
})

test('CLI builds before resolving a browser and reports build failure despite inherited prebuilt overrides', async () => {
  const root = await scratch()
  const buildFacts = join(root, 'build-facts.json')
  await executable(
    join(root, 'cargo'),
    `
await Bun.write(process.env.SDK_DESKTOP_FIXTURE_FAKE_BUILD_RECEIPT, JSON.stringify({
  args: process.argv.slice(2),
  buildDirectory: process.env.CARGO_BUILD_BUILD_DIR,
  debug: process.env.CARGO_PROFILE_DEV_DEBUG,
  jobs: process.env.CARGO_BUILD_JOBS,
}))
console.error('intentional source build failure')
process.exit(9)
`,
  )
  const out = join(root, 'results')
  const child = Bun.spawn({
    cmd: [
      process.execPath,
      join(import.meta.dir, 'run.ts'),
      '--case',
      'act-001',
      '--out',
      out,
    ],
    env: {
      ...process.env,
      PATH: `${root}:${process.env.PATH ?? ''}`,
      SDK_DESKTOP_FIXTURE_FAKE_BUILD_RECEIPT: buildFacts,
      CLAW_SDK_BINARY: '/unowned/prebuilt-sdk',
      SDK_DESKTOP_FIXTURE_BINARY: '/unowned/stale-sdk',
      BROWSEROS_BINARY: '/unavailable/browser',
    },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const [code, stderr] = await Promise.all([
    child.exited,
    new Response(child.stderr).text(),
  ])
  expect(code).toBe(1)
  expect(stderr).toContain('Source SDK build failed (9)')
  expect(stderr).not.toContain('Browser executable not found')
  const facts = jsonObject(
    await Bun.file(buildFacts).json(),
    'fake Cargo invocation',
  )
  expect(facts.args).toEqual([
    'build',
    '--locked',
    '-p',
    'neo-server',
    '--bin',
    'browseros-claw-sdk',
    '--message-format=json-render-diagnostics',
  ])
  expect(facts.debug).toBe('0')
  expect(facts.jobs).toBe('4')
  const build = jsonObject(
    await Bun.file(join(out, 'build.json')).json(),
    'build failure receipt',
  )
  expect(build.complete).toBe(false)
  expect(build.failure).toBe('Source SDK build failed (9)')
  expect(numberField(build, 'durationMs')).toBeGreaterThanOrEqual(0)
  await expect(access(stringField(facts, 'buildDirectory'))).rejects.toThrow()
  expect(await Bun.file(join(out, 'run.json')).json()).toMatchObject({
    complete: false,
    failure: 'Source SDK build failed (9)',
  })
  expect(await readFile(join(out, 'junit.xml'), 'utf8')).toContain(
    '<failure message="Source SDK build failed (9)"/>',
  )
})
