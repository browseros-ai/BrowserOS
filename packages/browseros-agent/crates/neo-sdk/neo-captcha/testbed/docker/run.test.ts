import { afterEach, expect, test } from 'bun:test'
import {
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { DEFAULT_KEY_DIR } from '../e2e/keys'
import { CommandError, keyMounts, parseArgs, runDockerE2e } from './run'

const dirs: string[] = []
afterEach(async () => {
  await Promise.all(
    dirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })),
  )
})
async function temporary() {
  const dir = await mkdtemp(join(tmpdir(), 'captcha-docker-test-'))
  dirs.push(dir)
  return dir
}

test('tests that runner flags and output paths survive argument parsing', () => {
  const options = parseArgs([
    '--',
    '--keys',
    'none',
    '--out',
    'my results',
    '--expect-tool-missing',
  ])
  expect(options.keys).toBe('none')
  expect(options.out).toEndWith('my results')
  expect(options.passthrough).toEqual(['--expect-tool-missing'])
  expect(parseArgs([])).toMatchObject({
    keys: 'placeholder',
    image: undefined,
    paidTaskLimit: undefined,
  })
  expect(parseArgs(['--keys', 'real']).paidTaskLimit).toBe(12)
  expect(
    parseArgs(['--keys', 'real', '--paid-task-limit', '4']).paidTaskLimit,
  ).toBe(4)
  expect(parseArgs(['--image', 'neo-sdk:test']).image).toBe('neo-sdk:test')
  expect(() => parseArgs(['--keys', 'invalid'])).toThrow('keys')
  expect(() => parseArgs(['--out'])).toThrow('value')
  expect(() => parseArgs(['--keep', '--clean'])).toThrow('keep')
  expect(() => parseArgs(['--clean', '--image', 'neo-sdk:test'])).toThrow(
    '--clean',
  )
  expect(() => parseArgs(['--paid-task-limit', '-1'])).toThrow('whole number')
})

test('tests that Docker owns the server and the fixture addresses', () => {
  for (const args of [
    ['--fixture-frame-host', 'other.test'],
    ['--fixture-frame-host=other.test'],
    ['--mcp-url', 'http://127.0.0.1:1/mcp'],
    ['--server', 'claw-server'],
  ])
    expect(() => parseArgs(args)).toThrow('owns')
})

test('tests that placeholder mode mounts one fake CapSolver key and blanks 2Captcha', async () => {
  const dir = await temporary()
  const args = await keyMounts('placeholder', join(dir, 'keys'), {}, dir)
  expect(args).toEqual([
    `--mount=type=bind,src=${join(dir, 'keys', 'capsolver-api-key')},dst=/run/neo-keys/capsolver-api-key,readonly`,
    '--env=BROWSEROS_CAPSOLVER_API_KEY_FILE=/run/neo-keys/capsolver-api-key',
    '--env=BROWSEROS_2CAPTCHA_API_KEY_FILE=',
  ])
  expect(await readFile(join(dir, 'keys', 'capsolver-api-key'), 'utf8')).toBe(
    'captcha-e2e-placeholder\n',
  )
  expect(await readdir(join(dir, 'keys'))).toEqual(['capsolver-api-key'])
})

test('tests that none blanks both keys, so a baked key is never read', async () => {
  const dir = await temporary()
  expect(await keyMounts('none', dir, {}, dir)).toEqual([
    '--env=BROWSEROS_CAPSOLVER_API_KEY_FILE=',
    '--env=BROWSEROS_2CAPTCHA_API_KEY_FILE=',
  ])
})

test('tests that real keys are mounted read-only without reading or copying them', async () => {
  const dir = await temporary()
  const source = join(dir, 'owner-key')
  await writeFile(source, 'fake-private-value')
  const args = await keyMounts(
    'real',
    join(dir, 'unused'),
    { BROWSEROS_CAPSOLVER_API_KEY_FILE: source },
    dir,
  )
  expect(args.join(' ')).toContain(
    `src=${source},dst=/run/neo-keys/capsolver-api-key,readonly`,
  )
  expect(args).toContain('--env=BROWSEROS_2CAPTCHA_API_KEY_FILE=')
  expect(args.join(' ')).not.toContain('fake-private-value')
  await expect(
    keyMounts(
      'real',
      dir,
      { BROWSEROS_CAPSOLVER_API_KEY_FILE: join(dir, 'missing-key') },
      dir,
    ),
  ).rejects.toThrow()
})

test('tests that real keys fall back to the default folder for both providers', async () => {
  const home = await temporary()
  await mkdir(join(home, DEFAULT_KEY_DIR), { recursive: true })
  await writeFile(join(home, DEFAULT_KEY_DIR, '2captcha-api-key'), 'fake')
  const args = await keyMounts('real', join(home, 'unused'), {}, home)
  expect(args).toEqual([
    '--env=BROWSEROS_CAPSOLVER_API_KEY_FILE=',
    `--mount=type=bind,src=${join(home, DEFAULT_KEY_DIR, '2captcha-api-key')},dst=/run/neo-keys/2captcha-api-key,readonly`,
    '--env=BROWSEROS_2CAPTCHA_API_KEY_FILE=/run/neo-keys/2captcha-api-key',
  ])
  await expect(
    keyMounts('real', join(home, 'unused'), {}, await temporary()),
  ).rejects.toThrow('found no CapSolver or 2Captcha key file')
})

const SHA = 'a'.repeat(64)
const SERVER_PATH =
  '/usr/lib/browserclaw/BrowserClawServer/default/resources/bin/browseros-claw-server'

/** A fake Docker, build script and runner, failing at the named step. */
function fakeCommands(failure: string) {
  const calls: string[][] = []
  const state = { container: '', dataDir: '' }
  const facts = () =>
    `${failure === 'kind' ? 'claw-server' : 'claw-sdk'}\n${JSON.stringify({
      revision: failure === 'revision' ? 'rev-0' : 'rev-1',
      server: { binary: 'browseros-claw-sdk', path: SERVER_PATH, sha256: SHA },
    })}`
  const inspect = () =>
    JSON.stringify([
      {
        State: {
          Status: 'running',
          Health: { Status: failure === 'health' ? 'unhealthy' : 'healthy' },
        },
        NetworkSettings: { Ports: { '9010/tcp': [{ HostPort: '40123' }] } },
      },
    ])
  const launch = (args: string[]) => {
    state.container = args[args.indexOf('--name') + 1] ?? ''
    state.dataDir =
      args
        .find((arg) => arg.includes('dst=/data'))
        ?.split('src=')[1]
        ?.split(',')[0] ?? ''
    return 'owned-container'
  }
  const docker = (args: string[]) => {
    if (args[1] === 'rm' && failure === 'cleanup')
      throw new Error('Docker removal failed')
    if (args[1] === 'run')
      return args.includes('--detach') ? launch(args) : facts()
    if (args[1] === 'inspect') return inspect()
    if (args[1] === 'exec')
      return `${failure === 'hash' ? 'b'.repeat(64) : SHA}  /proc/42/exe\n`
    return ''
  }
  const streamed: string[] = []
  const exec = async (input: string[], ...rest: unknown[]) => {
    if (rest[2] === true) streamed.push(input[0] ?? '')
    const args = input.flatMap((arg) => {
      const at = arg.startsWith('--') ? arg.indexOf('=') : -1
      return at < 0 ? [arg] : [arg.slice(0, at), arg.slice(at + 1)]
    })
    calls.push(args)
    if (args[0] === 'bash')
      return `${failure === 'tag' ? '' : 'TAG=neo-captcha-e2e:built\n'}IMAGE_ID=sha256:1`
    if (args[0] === 'git') return 'rev-1'
    if (args[0] === 'bun' && failure === 'runner')
      throw new CommandError(2, 'bun run failed')
    return args[0] === 'docker' ? docker(args) : ''
  }
  return { calls, state, exec, streamed }
}

async function harness(failure = '', extra: string[] = []) {
  const dir = await temporary()
  const out = join(dir, 'output')
  const fake = fakeCommands(failure)
  const execute = runDockerE2e(
    ['--keys', 'none', '--out', out, '--expect-tool-missing', ...extra],
    { env: {}, home: dir, exec: fake.exec, tools: async () => ['run'] },
  )
  return {
    execute,
    calls: fake.calls,
    streamed: fake.streamed,
    out,
    data: () => fake.state.dataDir,
    container: () => fake.state.container,
  }
}

const removed = (calls: string[][], name: string) =>
  calls.some((args) => args[1] === 'rm' && args.includes(name)) &&
  calls.some(
    (args) => args[1] === 'network' && args[2] === 'rm' && args.includes(name),
  )

test('tests that a healthy run builds the SDK image, proves its server and forwards the fixture addresses', async () => {
  const h = await harness()
  await h.execute
  const build = h.calls.find((args) => args[0] === 'bash') ?? []
  expect(build[1]).toBe('crates/neo-sdk/neo-server/docker/build.sh')
  const runner = h.calls.find((args) => args[0] === 'bun') ?? []
  expect(h.streamed).toEqual(['bun'])
  expect(runner).toContain('http://127.0.0.1:40123/mcp')
  expect(runner).not.toContain('--server')
  expect(runner).toContain('host.docker.internal')
  expect(
    runner.slice(
      runner.indexOf('--fixture-frame-host'),
      runner.indexOf('--fixture-frame-host') + 2,
    ),
  ).toEqual(['--fixture-frame-host', 'captcha-frame.test'])
  expect(runner).toContain('--expect-tool-missing')
  expect(
    runner.slice(runner.indexOf('--keys'), runner.indexOf('--keys') + 2),
  ).toEqual(['--keys', 'none'])
  const launch =
    h.calls.find(
      (args) =>
        args[0] === 'docker' && args[1] === 'run' && args.includes('--detach'),
    ) ?? []
  expect(launch).toContain('host.docker.internal:host-gateway')
  expect(launch).toContain('captcha-frame.test:host-gateway')
  expect(launch).toContain('NEO_MCP_REMOTE=true')
  expect(launch).toContain('BROWSEROS_CAPSOLVER_API_KEY_FILE=')
  expect(launch[launch.indexOf('--network') + 1]).toBe(h.container())
  expect(launch.at(-1)).toBe('neo-captcha-e2e:built')
  const proof = await Bun.file(join(h.out, 'proof.json')).json()
  expect(proof).toMatchObject({
    image: 'neo-captcha-e2e:built',
    kind: 'claw-sdk',
    tools: ['run'],
    succeeded: true,
  })
  expect(removed(h.calls, h.container())).toBe(true)
  await expect(stat(h.data())).rejects.toMatchObject({ code: 'ENOENT' })
})

test('tests that an existing image is used as given, without a build', async () => {
  const h = await harness('', ['--image', 'neo-sdk:prebuilt'])
  await h.execute
  expect(h.calls.some((args) => args[0] === 'bash')).toBe(false)
  expect(h.calls.some((args) => args[0] === 'git')).toBe(false)
  const launch = h.calls.find((args) => args.includes('--detach')) ?? []
  expect(launch.at(-1)).toBe('neo-sdk:prebuilt')
})

for (const failure of ['kind', 'revision', 'tag']) {
  test(`tests that an image failing the ${failure} proof never starts a container`, async () => {
    const h = await harness(failure)
    await expect(h.execute).rejects.toThrow()
    expect(h.calls.some((args) => args.includes('--detach'))).toBe(false)
    expect(h.calls.some((args) => args[0] === 'bun')).toBe(false)
  })
}

for (const failure of ['hash', 'health', 'runner']) {
  test(`tests that ${failure} failure cleans up the owned container and network`, async () => {
    const h = await harness(failure)
    await expect(h.execute).rejects.toThrow()
    expect(removed(h.calls, h.container())).toBe(true)
    if (failure !== 'runner')
      expect(h.calls.some((args) => args[0] === 'bun')).toBe(false)
  })
}

test('tests that the runner exit code survives for a setup blocker', async () => {
  const h = await harness('runner')
  const error = await h.execute.catch((caught) => caught)
  expect(error).toBeInstanceOf(CommandError)
  expect(error.exitCode).toBe(2)
})

test('tests that keep preserves data and clean removes only the built image', async () => {
  const kept = await harness('', ['--keep'])
  await kept.execute
  expect((await stat(kept.data())).isDirectory()).toBe(true)
  expect(kept.calls.some((args) => args[1] === 'rm')).toBe(false)
  dirs.push(kept.data().replace(/\/data$/, ''))
  const cleaned = await harness('', ['--clean'])
  await cleaned.execute
  expect(
    cleaned.calls.find((args) => args[1] === 'image' && args[2] === 'rm'),
  ).toEqual(['docker', 'image', 'rm', 'neo-captcha-e2e:built'])
})

test('tests that keep preserves a failed run for inspection', async () => {
  const h = await harness('hash', ['--keep'])
  await expect(h.execute).rejects.toThrow()
  expect(h.calls.some((args) => args[1] === 'rm')).toBe(false)
  dirs.push(h.data().replace(/\/data$/, ''))
})

test('tests that failed container removal preserves its backing directory and fails the run', async () => {
  const h = await harness('cleanup')
  await expect(h.execute).rejects.toThrow('cleanup')
  expect((await stat(h.data())).isDirectory()).toBe(true)
  dirs.push(h.data().replace(/\/data$/, ''))
})
