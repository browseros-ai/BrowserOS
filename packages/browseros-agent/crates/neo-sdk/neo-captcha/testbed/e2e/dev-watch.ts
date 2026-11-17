import { execFile, spawn } from 'node:child_process'
import { mkdir, mkdtemp, open, readFile, realpath, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { promisify } from 'node:util'
import { connected, fetchHealth, OWNER_PORTS } from './preflight'

export function parseWatchPorts(log: string) {
  const match = log.match(
    /Ports: CDP=(\d+) Server=(\d+) Extension=(\d+)(?:\s|$)/,
  )
  if (!match) return null
  const [cdp, server, extension] = [
    Number(match[1]),
    Number(match[2]),
    Number(match[3]),
  ]
  if ([cdp, server, extension].some((p) => !p || p > 65535)) return null
  return { cdp, server, extension }
}

function spawnWatch(env: NodeJS.ProcessEnv, fd: number) {
  const child = spawn('bun', ['run', 'dev:claw:watch:new'], {
    cwd: resolve(import.meta.dir, '../../../../..'),
    detached: true,
    env,
    stdio: ['ignore', fd, fd],
  })
  const exited = new Promise<void>((resolve, reject) => {
    child.once('exit', () => resolve())
    child.once('error', reject)
  })
  return { pid: child.pid, exited }
}

const defaults = {
  spawn: spawnWatch,
  health: fetchHealth,
  graceMs: 15_000,
  supervisor: async (pid: number) => {
    try {
      const { stdout } = await promisify(execFile)('pgrep', ['-P', String(pid)])
      const children = stdout.trim().split(/\s+/).map(Number)
      return children.length === 1 ? (children[0] ?? pid) : pid
    } catch (error) {
      if ((error as { code?: number }).code !== 1) throw error
      return pid
    }
  },
  signal: (pid: number, signal: NodeJS.Signals) => {
    try {
      process.kill(pid, signal)
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error
    }
  },
  processes: async () =>
    (
      await promisify(execFile)('ps', ['eww', '-axo', 'pid=,command='], {
        maxBuffer: 16 * 1024 * 1024,
      })
    ).stdout,
}

async function waitForExit(exited: Promise<unknown>, ms: number) {
  let timer: ReturnType<typeof setTimeout> | undefined
  try {
    await Promise.race([
      exited,
      new Promise((resolve) => {
        timer = setTimeout(resolve, ms)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

/**
 * `bun run dev:claw:watch:new` on the SDK backend (`BROWSERCLAW_USE_SDK=1`): this worktree's
 * browseros-claw-sdk under a fresh browser profile. Registers a handle immediately; waiting for
 * readiness must happen after ownership.
 */
export async function startDevWatch(
  options: {
    out: string
    startupTimeout: number
    serverEnv: NodeJS.ProcessEnv
  },
  overrides: Partial<typeof defaults> = {},
) {
  const dependencies = { ...defaults, ...overrides }
  const directory = await mkdtemp(
    join(await realpath(tmpdir()), 'captcha-dev-watch-'),
  )
  const logPath = join(options.out, 'dev-watch.log')
  let child: ReturnType<typeof spawnWatch>
  try {
    await mkdir(options.out, { recursive: true })
    await mkdir(join(directory, 'state'))
    const log = await open(logPath, 'w')
    try {
      child = dependencies.spawn(
        {
          ...process.env,
          ...options.serverEnv,
          BROWSERCLAW_USE_SDK: '1',
          BROWSERCLAW_HOSTED_MODE: '1',
          BROWSERCLAW_DIR: join(directory, 'state'),
          TMPDIR: directory,
          CARGO_TARGET_DIR:
            // biome-ignore lint/suspicious/noUndeclaredEnvVars: this standalone test runner is not a Turbo task.
            process.env.CARGO_TARGET_DIR ||
            '/Volumes/extssd/cargo-target/captcha-e2e-dev-watch',
        },
        log.fd,
      )
    } finally {
      await log.close()
    }
  } catch (error) {
    await rm(directory, { recursive: true, force: true })
    throw error
  }
  let finished = false
  let stopped: Promise<void> | undefined
  const exited = child.exited
    .catch(() => {})
    .finally(() => {
      finished = true
    })
  const deadline = Date.now() + options.startupTimeout
  return {
    directory,
    logPath,
    async ready() {
      let waiting = 'Ports: CDP=<c> Server=<s> Extension=<e>'
      do {
        if (stopped) throw new Error('dev-watch interrupted')
        if (finished)
          throw new Error(`dev-watch exited before ready; see ${logPath}`)
        const ports = parseWatchPorts(await readFile(logPath, 'utf8'))
        if (ports) {
          for (const port of Object.values(ports))
            if (OWNER_PORTS.has(port))
              throw new Error(`refusing owner port ${port}`)
          const url = `http://127.0.0.1:${ports.server}/mcp`
          waiting = 'health status ok and browser connected'
          try {
            if (connected(await dependencies.health(url))) return url
          } catch {
            /* The server may still be compiling. */
          }
        }
        await Bun.sleep(Math.min(250, Math.max(1, deadline - Date.now())))
      } while (Date.now() < deadline)
      throw new Error(
        `dev-watch startup timed out after ${options.startupTimeout} ms waiting for ${waiting}; see ${logPath}`,
      )
    },
    stop() {
      stopped ??= (async () => {
        if (child.pid && !finished) {
          const supervisor = await dependencies.supervisor(child.pid)
          // Bun forwards signals: hitting its whole group sends SIGINT twice.
          if (!finished) dependencies.signal(supervisor, 'SIGINT')
          await waitForExit(exited, dependencies.graceMs)
          if (!finished) {
            dependencies.signal(-child.pid, 'SIGKILL')
            await waitForExit(exited, 3000)
          }
        }
        const deadline = Date.now() + dependencies.graceMs
        let leaks: string[] = []
        while (true) {
          leaks = (await dependencies.processes())
            .split('\n')
            .filter((line) => line.includes(directory))
            .map((line) => line.trim().split(/\s+/)[0] ?? '')
          if (!leaks.length || Date.now() >= deadline) break
          await Bun.sleep(Math.min(100, Math.max(1, deadline - Date.now())))
        }
        if (leaks.length)
          throw new Error(
            `dev-watch cleanup failed: leftover PIDs ${leaks.join(', ')} reference ${directory}`,
          )
        await rm(directory, { recursive: true, force: true })
      })()
      return stopped
    },
  }
}
