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
import { keyMounts, parseArgs, runDockerE2e } from './run'

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
  expect(parseArgs([]).keys).toBe('placeholder')
  expect(() => parseArgs(['--keys', 'invalid'])).toThrow('keys')
  expect(() => parseArgs(['--out'])).toThrow('value')
  expect(() => parseArgs(['--keep', '--clean'])).toThrow('keep')
})

test('tests that placeholder mode mounts only a fake CapSolver key', async () => {
  const dir = await temporary()
  const args = await keyMounts('placeholder', dir, {})
  expect(args.join(' ')).toContain('readonly')
  expect(args.join(' ')).toContain(
    'BROWSEROS_CAPSOLVER_API_KEY_FILE=/run/captcha/capsolver-api-key',
  )
  expect(await readFile(join(dir, 'capsolver-api-key'), 'utf8')).toBe(
    'captcha-e2e-placeholder\n',
  )
  expect(await readdir(dir)).toEqual(['capsolver-api-key'])
  expect(await keyMounts('none', dir, {})).toEqual([])
})

test('tests that real keys are mounted without reading or copying their contents', async () => {
  const dir = await temporary()
  const source = join(dir, 'owner-key')
  await writeFile(source, 'fake-private-value')
  const env = {
    BROWSEROS_CAPSOLVER_API_KEY_FILE: source,
  }
  const args = await keyMounts('real', join(dir, 'unused'), env)
  expect(args.join(' ')).toContain(
    `src=${source},dst=/run/captcha/capsolver-api-key,readonly`,
  )
  expect(args.join(' ')).not.toContain('fake-private-value')
  expect(args.filter((arg) => arg.startsWith('--mount'))).toHaveLength(1)
  await expect(
    keyMounts('real', dir, {
      ...env,
      BROWSEROS_CAPSOLVER_API_KEY_FILE: '/missing-key',
    }),
  ).rejects.toThrow()
})

test('tests that real mode forwards other configured key-file variables as-is', async () => {
  const dir = await temporary()
  const source = join(dir, 'fake-key')
  await writeFile(source, 'fake-private-value')
  const args = await keyMounts('real', dir, {
    BROWSEROS_CAPSOLVER_API_KEY_FILE: source,
    BROWSEROS_EXAMPLE_API_KEY_FILE: source,
    UNRELATED_API_KEY_FILE: '/unused',
  })
  expect(args.join(' ')).toContain(`src=${source},dst=${source},readonly`)
  expect(args.join(' ')).toContain(`BROWSEROS_EXAMPLE_API_KEY_FILE=${source}`)
  expect(args.filter((arg) => arg.startsWith('--mount'))).toHaveLength(2)
  expect(args.join(' ')).not.toContain('fake-private-value')
  expect(args.join(' ')).not.toContain('UNRELATED_API_KEY_FILE')
})

test('tests that an explicitly configured extra key file must exist', async () => {
  const dir = await temporary()
  const source = join(dir, 'fake-key')
  await writeFile(source, 'fake-private-value')
  await expect(
    keyMounts('real', dir, {
      BROWSEROS_CAPSOLVER_API_KEY_FILE: source,
      BROWSEROS_EXAMPLE_API_KEY_FILE: join(dir, 'missing'),
    }),
  ).rejects.toThrow()
})

async function harness(failure = '', extra: string[] = []) {
  const dir = await temporary()
  const target = join(dir, 'target')
  const binary = join(
    target,
    'aarch64-unknown-linux-gnu/release/browseros-claw-server-rs',
  )
  await mkdir(join(target, 'aarch64-unknown-linux-gnu/release'), {
    recursive: true,
  })
  await writeFile(binary, 'test-binary')
  const hash = new Bun.CryptoHasher('sha256')
    .update('test-binary')
    .digest('hex')
  const calls: string[][] = []
  let dataDir = ''
  let container = ''
  const out = join(dir, 'output')
  const execute = runDockerE2e(
    ['--keys', 'none', '--out', out, '--expect-tool-missing', ...extra],
    {
      env: { CARGO_TARGET_DIR: target },
      exec: async (input: string[]) => {
        const args = input.flatMap((arg) => {
          const at = arg.startsWith('--') ? arg.indexOf('=') : -1
          return at < 0 ? [arg] : [arg.slice(0, at), arg.slice(at + 1)]
        })
        calls.push(args)
        if (args[1] === 'rm' && failure === 'cleanup')
          throw new Error('Docker removal failed')
        if (args[1] === 'run' && args.includes('--detach')) {
          container = args[args.indexOf('--name') + 1] ?? ''
          dataDir =
            args
              .find((arg) => arg.includes('dst=/data'))
              ?.split('src=')[1]
              ?.split(',')[0] ?? ''
          return 'owned-container'
        }
        if (args[1] === 'inspect') {
          return JSON.stringify([
            {
              State: {
                Status: 'running',
                Health: {
                  Status: failure === 'health' ? 'unhealthy' : 'healthy',
                },
              },
              NetworkSettings: {
                Ports: {
                  '9010/tcp': [{ HostPort: '40123' }],
                  '6080/tcp': [{ HostPort: '40124' }],
                },
              },
            },
          ])
        }
        if (
          args[1] === 'exec' &&
          failure === 'permission' &&
          !args.includes('neo')
        )
          throw new Error('Permission denied reading a different user process')
        if (args[1] === 'exec')
          return `${failure === 'hash' ? 'wrong' : hash}  /proc/42/exe\n`
        if (args[1] === 'run' && args.includes('--entrypoint'))
          return '/usr/lib/browserclaw/BrowserClawServer/default/resources/bin/browseros-claw-server\n'
        if (args[0] === 'bun' && failure === 'runner')
          throw new Error('runner failed')
        return ''
      },
      tools: async () => ['tabs', 'snapshot'],
    },
  )
  return {
    execute,
    calls,
    out,
    data: () => dataDir,
    container: () => container,
  }
}

test('tests that a healthy run proves the overlay and forwards the Docker fixture address', async () => {
  const h = await harness()
  await h.execute
  const runner = h.calls.find((args) => args[0] === 'bun') ?? []
  expect(runner).toContain('http://127.0.0.1:40123/mcp')
  expect(runner).toContain('0.0.0.0')
  expect(runner).toContain('host.docker.internal')
  expect(runner).toContain('--expect-tool-missing')
  expect(
    runner.slice(runner.indexOf('--keys'), runner.indexOf('--keys') + 2),
  ).toEqual(['--keys', 'none'])
  expect(
    h.calls.some((args) => args[1] === 'rm' && args.includes(h.container())),
  ).toBe(true)
  expect(await Bun.file(join(h.out, 'proof.json')).exists()).toBe(true)
  await expect(stat(h.data())).rejects.toMatchObject({ code: 'ENOENT' })
})

for (const failure of ['hash', 'health', 'runner']) {
  test(`tests that ${failure} failure cleans up the owned container`, async () => {
    const h = await harness(failure)
    await expect(h.execute).rejects.toThrow()
    expect(
      h.calls.some((args) => args[1] === 'rm' && args.includes(h.container())),
    ).toBe(true)
    if (failure !== 'runner')
      expect(h.calls.some((args) => args[0] === 'bun')).toBe(false)
  })
}

test('tests that keep preserves data and clean removes only harness images', async () => {
  const kept = await harness('', ['--keep'])
  await kept.execute
  expect((await stat(kept.data())).isDirectory()).toBe(true)
  expect(kept.calls.some((args) => args[1] === 'rm')).toBe(false)
  dirs.push(kept.data().replace(/\/data$/, ''))
  const cleaned = await harness('', ['--clean'])
  await cleaned.execute
  expect(
    cleaned.calls.some((args) => args[1] === 'image' && args[2] === 'rm'),
  ).toBe(true)
})

test('tests that keep preserves a failed run for inspection', async () => {
  const h = await harness('hash', ['--keep'])
  await expect(h.execute).rejects.toThrow()
  expect(h.calls.some((args) => args[1] === 'rm')).toBe(false)
  dirs.push(h.data().replace(/\/data$/, ''))
})

test('tests that the running binary is hashed as its owning user', async () => {
  const h = await harness('permission')
  await h.execute
  expect((await Bun.file(join(h.out, 'proof.json')).json()).succeeded).toBe(
    true,
  )
})

test('tests that failed container removal preserves its backing directory and fails the run', async () => {
  const h = await harness('cleanup')
  await expect(h.execute).rejects.toThrow('cleanup')
  expect((await stat(h.data())).isDirectory()).toBe(true)
  dirs.push(h.data().replace(/\/data$/, ''))
})
