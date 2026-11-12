import { expect, test } from 'bun:test'
import { writeFileSync } from 'node:fs'
import { mkdtemp, rm, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { parseWatchPorts, startDevWatch } from './dev-watch'

const health = { status: 'ok', browser: { connected: true } }

test('tests that watch ports are parsed from complete, decorated log lines', () => {
  expect(
    parseWatchPorts(
      '\u001b[36m[info]\u001b[0m Ports: CDP=9401 Server=9402 Extension=9403\n',
    ),
  ).toEqual({
    cdp: 9401,
    server: 9402,
    extension: 9403,
  })
  for (const log of [
    'Ports: CDP=9401 Server=',
    'unrelated Server=9402',
    'Ports: CDP=0 Server=9402 Extension=9403',
  ])
    expect(parseWatchPorts(log)).toBeNull()
})

test('tests that a fake watch runs the SDK backend with isolated state, logs, and idempotent teardown', async () => {
  const out = await mkdtemp(join(tmpdir(), 'watch-test-'))
  const signals: Array<[number, string]> = []
  let env: NodeJS.ProcessEnv = {}
  let exit!: () => void
  const exited = new Promise<void>((resolve) => {
    exit = resolve
  })
  const watch = await startDevWatch(
    {
      out,
      startupTimeout: 100,
      serverEnv: {
        BROWSEROS_CAPSOLVER_API_KEY_FILE: '/fake/key',
        BROWSEROS_CAPTCHA_PAID_TASK_LIMIT: '3',
        BROWSERCLAW_HOSTED_MODE: '0',
        BROWSERCLAW_USE_SDK: '',
      },
    },
    {
      spawn: (childEnv, fd) => {
        env = childEnv
        writeFileSync(fd, 'Ports: CDP=9401 Server=9402 Extension=9403\n')
        return { pid: 12345, exited }
      },
      health: async () => health,
      supervisor: async () => 12346,
      signal: (pid, signal) => {
        signals.push([pid, signal])
        exit()
      },
      processes: async () => '',
    },
  )
  try {
    expect(await watch.ready()).toBe('http://127.0.0.1:9402/mcp')
    expect(env.BROWSERCLAW_DIR).toStartWith(watch.directory)
    expect(env.TMPDIR).toBe(watch.directory)
    expect(env.BROWSEROS_CAPSOLVER_API_KEY_FILE).toBe('/fake/key')
    expect(env.BROWSEROS_CAPTCHA_PAID_TASK_LIMIT).toBe('3')
    expect(env.BROWSERCLAW_USE_SDK).toBe('1')
    expect(env.BROWSERCLAW_HOSTED_MODE).toBe('1')
    expect(env.CARGO_TARGET_DIR).toBeTruthy()
    await watch.stop()
    await watch.stop()
    expect(signals).toEqual([[12346, 'SIGINT']])
    expect(await Bun.file(join(out, 'dev-watch.log')).text()).toContain(
      'Server=9402',
    )
    await expect(stat(watch.directory)).rejects.toThrow()
  } finally {
    await rm(out, { recursive: true, force: true })
  }
})

test('tests that startup timeout and leaked processes fail clearly with fake process supervision', async () => {
  const out = await mkdtemp(join(tmpdir(), 'watch-failure-test-'))
  let directory = ''
  const watch = await startDevWatch(
    { out, startupTimeout: 10, serverEnv: {} },
    {
      spawn: (env) => {
        directory = String(env.TMPDIR)
        return { pid: 12345, exited: Promise.resolve() }
      },
      health: async () => health,
      supervisor: async () => 12346,
      graceMs: 5,
      signal: () => {},
      processes: async () =>
        `56789 browser --user-data-dir=${directory}/browseros-dev-test`,
    },
  )
  try {
    await expect(watch.ready()).rejects.toThrow(/Ports:|exited/)
    await expect(watch.stop()).rejects.toThrow('56789')
    expect((await stat(directory)).isDirectory()).toBe(true)
  } finally {
    await rm(directory, { recursive: true, force: true })
    await rm(out, { recursive: true, force: true })
  }
})

test('tests that teardown waits for exiting descendants before declaring a leak', async () => {
  const out = await mkdtemp(join(tmpdir(), 'watch-settle-test-'))
  let root = ''
  let scans = 0
  const watch = await startDevWatch(
    { out, startupTimeout: 10, serverEnv: {} },
    {
      spawn: (env) => {
        root = String(env.TMPDIR)
        return { pid: 12345, exited: Promise.resolve() }
      },
      supervisor: async () => 12346,
      signal: () => {},
      graceMs: 100,
      processes: async () =>
        ++scans === 1 ? `56789 browser --user-data-dir=${root}/profile` : '',
    },
  )
  try {
    await watch.stop()
    expect(scans).toBeGreaterThan(1)
    await expect(stat(root)).rejects.toThrow()
  } finally {
    await rm(root, { recursive: true, force: true })
    await rm(out, { recursive: true, force: true })
  }
})

test('tests that a startup timeout still tears down and an unresponsive supervisor gets group SIGKILL', async () => {
  const out = await mkdtemp(join(tmpdir(), 'watch-timeout-test-'))
  const signals: Array<[number, string]> = []
  let exit!: () => void
  const exited = new Promise<void>((resolve) => {
    exit = resolve
  })
  const watch = await startDevWatch(
    { out, startupTimeout: 5, serverEnv: {} },
    {
      spawn: () => ({ pid: 12345, exited }),
      supervisor: async () => 12346,
      graceMs: 5,
      signal: (pid, signal) => {
        signals.push([pid, signal])
        if (signal === 'SIGKILL') exit()
      },
      processes: async () => '',
    },
  )
  try {
    await expect(watch.ready()).rejects.toThrow('waiting for Ports:')
    await watch.stop()
    expect(signals).toEqual([
      [12346, 'SIGINT'],
      [-12345, 'SIGKILL'],
    ])
    await expect(stat(watch.directory)).rejects.toThrow()
  } finally {
    await rm(out, { recursive: true, force: true })
  }
})
