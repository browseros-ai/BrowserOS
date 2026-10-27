import { type ChildProcess, spawn } from 'node:child_process'
import { constants } from 'node:fs'
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises'
import { createServer } from 'node:net'
import { join, resolve } from 'node:path'

const SERVER_ROOT = resolve(import.meta.dir, '../..')
const LOG_LIMIT = 16_384

/** Owns one POSIX process group, including same-group descendants.
 * Windows needs a job-object implementation; do not silently use PID-only cleanup. */
export class OwnedProcess {
  readonly child: ChildProcess
  private output = ''
  private error: Error | undefined
  private group: number | undefined
  private readonly spawned: Promise<void>
  private stopping: Promise<void> | undefined

  constructor(command: string, args: string[], env: NodeJS.ProcessEnv) {
    if (process.platform === 'win32') {
      throw new Error(
        'Owned E2E process groups require POSIX; Windows is not supported.',
      )
    }
    this.child = spawn(command, args, {
      cwd: SERVER_ROOT,
      detached: true,
      env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    this.spawned = new Promise((done) => {
      this.child.once('spawn', () => {
        this.group = this.child.pid
        done()
      })
      this.child.on('error', (error) => {
        this.error = error
        done()
      })
    })
    for (const stream of [this.child.stdout, this.child.stderr]) {
      stream?.on('data', (chunk: Buffer) => {
        this.output = (this.output + chunk.toString()).slice(-LOG_LIMIT)
      })
    }
  }

  assertRunning(): void {
    if (
      this.error ||
      this.child.exitCode !== null ||
      this.child.signalCode !== null
    ) {
      throw new Error(
        `Owned process failed: ${this.error?.message ?? `exit=${this.child.exitCode}, signal=${this.child.signalCode}`}\n${this.output}`,
      )
    }
  }

  diagnostics(): string {
    return this.output
  }

  stop(): Promise<void> {
    // Publish before work starts, so concurrent teardown shares one final attempt.
    this.stopping ??= Promise.resolve().then(() => this.stopGroup())
    return this.stopping
  }

  private signalGroup(signal: NodeJS.Signals | 0): boolean {
    if (this.group === undefined) return false
    try {
      process.kill(-this.group, signal)
      return true
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ESRCH') {
        // Never signal this numeric group again once its disappearance is observed.
        this.group = undefined
        return false
      }
      throw new Error(
        `Failed to signal owned group ${this.group} (${signal}): ${String(error)}`,
      )
    }
  }

  private exited(): boolean {
    return (
      this.child.exitCode !== null ||
      this.child.signalCode !== null ||
      !!this.error
    )
  }

  private async stopGroup(): Promise<void> {
    await this.spawned
    const failures: unknown[] = []
    try {
      try {
        this.signalGroup('SIGTERM')
      } catch (error) {
        failures.push(error)
      }
      const deadline = Date.now() + 1_500
      while (Date.now() < deadline) {
        let present = false
        try {
          present = this.signalGroup(0)
        } catch (error) {
          failures.push(error)
          break
        }
        if (!present && this.exited()) break
        await Bun.sleep(25)
      }
      // Leader exit is not descendant exit. Always escalate any still-owned group.
      try {
        this.signalGroup('SIGKILL')
      } catch (error) {
        failures.push(error)
      }
      // Retire responsibility after this final attempt, even on signalling failure.
      this.group = undefined
      this.child.stdout?.destroy()
      this.child.stderr?.destroy()
      const exitDeadline = Date.now() + 1_500
      while (!this.exited() && Date.now() < exitDeadline) await Bun.sleep(25)
      if (!this.exited())
        failures.push(
          new Error(
            `Owned leader ${this.child.pid} did not exit after SIGKILL`,
          ),
        )
    } finally {
      this.group = undefined
    }
    if (failures.length)
      throw new AggregateError(failures, 'Owned process-group teardown failed')
  }
}

export async function waitUntil(
  label: string,
  processes: OwnedProcess[],
  ready: () => Promise<boolean>,
  timeoutMs = 30_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    for (const process of processes) process.assertRunning()
    if (await ready()) {
      for (const process of processes) process.assertRunning()
      return
    }
    await Bun.sleep(50)
  }
  throw new Error(
    `Timed out waiting for ${label}\n${processes.map((process) => process.diagnostics()).join('\n')}`,
  )
}

/** No inherited credentials, config overrides, telemetry keys or proxy settings. */
export async function isolatedEnvironment(
  root: string,
): Promise<NodeJS.ProcessEnv> {
  const dirs = {
    HOME: 'home',
    XDG_CONFIG_HOME: 'config',
    XDG_DATA_HOME: 'data',
    XDG_CACHE_HOME: 'cache',
    BROWSEROS_DIR: 'browseros',
    TMPDIR: 'tmp',
  }
  const env: NodeJS.ProcessEnv = {
    PATH: process.env.PATH,
    NODE_ENV: 'test',
    LANG: 'C.UTF-8',
  }
  for (const [key, name] of Object.entries(dirs)) {
    env[key] = join(root, name)
    await mkdir(env[key], { recursive: true })
  }
  return env
}

export async function reservePort() {
  const server = createServer()
  await new Promise<void>((done, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', done)
  })
  const address = server.address()
  if (!address || typeof address === 'string')
    throw new Error('No TCP port allocated')
  return {
    port: address.port,
    release: () =>
      new Promise<void>((done, reject) =>
        server.close((error) => (error ? reject(error) : done())),
      ),
  }
}

export async function requireBrowserBinary(
  value = process.env.BROWSEROS_BINARY,
): Promise<string> {
  if (!value?.trim())
    throw new Error(
      'BROWSEROS_BINARY must point to a real BrowserOS executable (stock Chromium is not supported).',
    )
  const binary = resolve(value)
  try {
    await access(binary, constants.X_OK)
  } catch {
    throw new Error(`BROWSEROS_BINARY is not executable: ${binary}`)
  }
  return binary
}

async function healthy(url: string): Promise<boolean> {
  try {
    return (await fetch(url, { signal: AbortSignal.timeout(500) })).ok
  } catch {
    return false
  }
}

/** Own all resources before starting; partial startup follows the same teardown. */
export async function withE2e<T>(
  run: (baseUrl: string) => Promise<T>,
  binaryPath = process.env.BROWSEROS_BINARY,
): Promise<T> {
  const binary = await requireBrowserBinary(binaryPath)
  // Chromium appends its own Unix socket path; nested CI TMPDIRs exceed sun_path.
  // A private mkdtemp directory under /tmp keeps it short without sharing state.
  const root = await mkdtemp('/tmp/bos-e2e-')
  const processes: OwnedProcess[] = []
  let reservation: Awaited<ReturnType<typeof reservePort>> | undefined
  let result: T | undefined
  const failures: unknown[] = []
  try {
    const env = await isolatedEnvironment(root)
    const profile = join(root, 'profile')
    await mkdir(profile)
    const browser = new OwnedProcess(
      binary,
      [
        '--headless=new',
        '--no-first-run',
        '--no-default-browser-check',
        '--disable-browseros-server',
        '--disable-browseros-extensions',
        '--remote-debugging-address=127.0.0.1',
        '--remote-debugging-port=0',
        `--user-data-dir=${profile}`,
        'about:blank',
      ],
      env,
    )
    processes.push(browser)
    let cdpPort = 0
    await waitUntil('owned browser CDP', processes, async () => {
      try {
        const port = Number(
          (await readFile(join(profile, 'DevToolsActivePort'), 'utf8')).split(
            '\n',
          )[0],
        )
        if (!Number.isInteger(port) || port < 1 || port > 65535) return false
        if (!(await healthy(`http://127.0.0.1:${port}/json/version`)))
          return false
        cdpPort = port
        return true
      } catch {
        return false
      }
    })
    reservation = await reservePort()
    const serverPort = reservation.port
    const baseUrl = `http://127.0.0.1:${serverPort}`
    const config = join(root, 'sidecar.json')
    await writeFile(
      config,
      JSON.stringify({
        ports: { server: serverPort, cdp: cdpPort },
        directories: { resources: root, execution: join(root, 'logs') },
        flags: { allow_remote_in_mcp: false },
        instance: {},
      }),
    )
    await reservation.release()
    reservation = undefined
    const server = new OwnedProcess(
      process.execPath,
      [join(SERVER_ROOT, 'src/index.ts'), '--config', config],
      env,
    )
    processes.push(server)
    await waitUntil(
      'owned server health and discovery',
      processes,
      async () => {
        try {
          // A healthy unrelated port owner cannot create this private discovery file.
          const discovery = JSON.parse(
            await readFile(join(root, 'browseros/server.json'), 'utf8'),
          )
          return (
            discovery.server_port === serverPort &&
            discovery.cdp_port === cdpPort &&
            (await healthy(`${baseUrl}/system/health`))
          )
        } catch {
          return false
        }
      },
    )
    result = await run(baseUrl)
  } catch (error) {
    failures.push(error)
  }
  try {
    await reservation?.release()
  } catch (error) {
    failures.push(error)
  }
  let stopped = true
  for (const process of processes.reverse()) {
    try {
      await process.stop()
    } catch (error) {
      stopped = false
      failures.push(error)
    }
  }
  // Do not remove a profile while a process may still be using it.
  if (stopped) {
    try {
      await rm(root, { recursive: true, force: true })
    } catch (error) {
      failures.push(error)
    }
  }
  if (failures.length === 1) throw failures[0]
  if (failures.length)
    throw new AggregateError(failures, `E2E failed; temporary root: ${root}`)
  return result as T
}
