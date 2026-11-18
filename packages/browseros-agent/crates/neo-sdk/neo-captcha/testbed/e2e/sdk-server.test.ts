import { afterEach, expect, test } from 'bun:test'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  buildSdkServer,
  prepareSdkServer,
  sdkBinary,
  startSdkServer,
} from './sdk-server'

const dirs: string[] = []
afterEach(async () => {
  await Promise.all(
    dirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })),
  )
})

test('tests that the SDK binary override wins and Cargo target directories are honored', () => {
  expect(
    sdkBinary({
      CLAW_SDK_BINARY: '/tmp/prebuilt-sdk',
      CARGO_TARGET_DIR: '/tmp/cargo-test',
    }),
  ).toBe('/tmp/prebuilt-sdk')
  expect(sdkBinary({ CARGO_TARGET_DIR: '/tmp/cargo-test' })).toBe(
    '/tmp/cargo-test/debug/browseros-claw-sdk',
  )
  expect(sdkBinary({})).toEndWith(
    '/packages/browseros-agent/target/debug/browseros-claw-sdk',
  )
})

test('tests that the build compiles neo-server, not claw-server', () => {
  let command: string[] = []
  buildSdkServer(((options: { cmd: string[] }) => {
    command = options.cmd
    return { exitCode: 0 }
  }) as unknown as typeof Bun.spawnSync)
  expect(command).toEqual([
    'cargo',
    'build',
    '--locked',
    '-p',
    'neo-server',
    '--bin',
    'browseros-claw-sdk',
  ])
})

/** Stands in for browseros-claw-sdk: reads its launch file, records its environment, serves ready. */
const FAKE_SERVER = `
const config = JSON.parse(await Bun.file(process.argv[3]).text())
console.log(JSON.stringify({ config, env: {
  key: process.env.BROWSEROS_CAPSOLVER_API_KEY_FILE ?? null,
  other: process.env.BROWSEROS_2CAPTCHA_API_KEY_FILE ?? null,
  limit: process.env.BROWSEROS_CAPTCHA_PAID_TASK_LIMIT ?? null,
  hosted: process.env.BROWSERCLAW_HOSTED_MODE,
  state: process.env.BROWSERCLAW_DIR,
  home: process.env.HOME,
  hostToken: process.env.CLAW_HOST_TOKEN_FILE ?? null,
} }))
Bun.serve({ hostname: '127.0.0.1', port: config.ports.server, fetch: () => new Response('ready') })
`

test('tests that the server gets a launch file, private state and only the configured keys', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'captcha-sdk-server-test-'))
  dirs.push(dir)
  const script = join(dir, 'fake-sdk.ts')
  await writeFile(script, FAKE_SERVER)
  const binary = join(dir, 'fake-sdk')
  await writeFile(
    binary,
    `#!/bin/sh\nexec '${process.execPath}' '${script}' "$@"\n`,
    { mode: 0o700 },
  )
  const logPath = join(dir, 'sdk-server.log')
  const server = await startSdkServer(9333, {
    binary,
    logPath,
    env: {
      BROWSEROS_CAPSOLVER_API_KEY_FILE: '/fake/key',
      BROWSEROS_2CAPTCHA_API_KEY_FILE: undefined,
      BROWSEROS_CAPTCHA_PAID_TASK_LIMIT: '2',
      CLAW_HOST_TOKEN_FILE: '/owner/host-token',
    },
  })
  expect(await (await fetch(`${server.baseUrl}/system/ready`)).text()).toBe(
    'ready',
  )
  await server.stop()
  const seen = JSON.parse(
    (await readFile(logPath, 'utf8')).split('\n')[0] ?? '',
  )
  expect(seen.config).toMatchObject({
    ports: { server: Number(new URL(server.baseUrl).port), cdp: 9333 },
    directories: { resources: 'resources' },
  })
  expect(seen.env).toMatchObject({
    key: '/fake/key',
    other: null,
    limit: '2',
    hosted: '1',
    hostToken: null,
  })
  expect(seen.env.state).toStartWith(tmpdir().replace(/\/$/, ''))
  expect(await Bun.file(seen.env.state).exists()).toBe(false)
})

test('tests that a prebuilt binary skips the Cargo build', () => {
  let builds = 0
  prepareSdkServer({ CLAW_SDK_BINARY: '/tmp/prebuilt-sdk' }, () => {
    builds++
  })
  expect(builds).toBe(0)
  prepareSdkServer({}, () => {
    builds++
  })
  expect(builds).toBe(1)
})

test('tests that a server that exits early fails with its log path', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'captcha-sdk-server-test-'))
  dirs.push(dir)
  const binary = join(dir, 'crashing-sdk')
  await writeFile(binary, '#!/bin/sh\necho boom >&2\nexit 3\n', { mode: 0o700 })
  const logPath = join(dir, 'sdk-server.log')
  await expect(
    startSdkServer(9333, { binary, logPath, env: {} }),
  ).rejects.toThrow(`browseros-claw-sdk exited with 3; see ${logPath}`)
  expect(await readFile(logPath, 'utf8')).toContain('boom')
})
