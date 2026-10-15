// biome-ignore-all lint/suspicious/noUndeclaredEnvVars: Standalone compatibility CLIs are never Turbo-cached.
/** Owns an isolated server/browser process tree from config creation through bounded, awaited teardown. */
import { type ChildProcess, execFile, spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { createServer } from 'node:net'
import { homedir, tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { promisify } from 'node:util'

export const workspace = resolve(import.meta.dir, '../..')
export const trustedOrigin =
  'chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk'
export type ServerKind = 'ts' | 'rust'
export interface BootedServer {
  kind: ServerKind
  url: string
  root: string
  cdpPort: number
  stop(): Promise<void>
}
const cleanups = new Set<() => Promise<void>>()

export async function freePort(): Promise<number> {
  const server = createServer()
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  const address = server.address()
  if (!address || typeof address === 'string')
    throw new Error('No allocated port')
  await new Promise<void>((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve())),
  )
  return address.port
}

/** Spawn a private POSIX process group: killing this group never targets a port owner or another run. */
function ownedProcess(
  command: string[],
  env: NodeJS.ProcessEnv,
  log: string[],
) {
  const child = spawn(command[0], command.slice(1), {
    cwd: workspace,
    env,
    detached: process.platform !== 'win32',
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  const append = (data: Buffer) => {
    log.push(data.toString())
    while (log.join('').length > 100_000) log.shift()
  }
  child.stdout?.on('data', append)
  child.stderr?.on('data', append)
  let spawnError: Error | undefined
  child.once('error', (error) => {
    spawnError = error
  })
  const exited = new Promise<void>((resolve) => {
    child.once('exit', () => resolve())
    child.once('error', () => resolve())
  })
  return { child, exited, error: () => spawnError }
}

async function terminate(child: ChildProcess, exited: Promise<void>) {
  const pid = child.pid
  if (!pid) return
  if (process.platform === 'win32') {
    // taskkill's /T includes owned browser helpers. No image-name or port-wide termination is used.
    if (child.exitCode === null && child.signalCode === null)
      await promisify(execFile)('taskkill', ['/PID', String(pid), '/T', '/F'])
    await exited
    return
  }
  const liveGroup = async () => {
    const { stdout } = await promisify(execFile)('ps', [
      '-axo',
      'pgid=,uid=,stat=',
    ])
    const members = stdout
      .trim()
      .split('\n')
      .map((line) => line.trim().split(/\s+/))
      .filter(
        ([group, , state]) => Number(group) === pid && !state.startsWith('Z'),
      )
    if (members.some(([, uid]) => Number(uid) !== process.getuid?.()))
      throw new Error(
        `Owned process group ${pid} contains a different uid; refusing to signal`,
      )
    return members.length > 0
  }
  const signal = async (name: NodeJS.Signals) => {
    if (!(await liveGroup())) return
    try {
      process.kill(-pid, name)
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code
      // Darwin can return EPERM when the last live member exits between ps and kill (zombies remain).
      if (code !== 'ESRCH' && !(code === 'EPERM' && !(await liveGroup())))
        throw error
    }
  }
  await signal('SIGTERM')
  await Promise.race([exited, Bun.sleep(3_000)])
  await signal('SIGKILL')
  await exited
  for (let attempt = 0; attempt < 100; attempt++) {
    if (!(await liveGroup())) return
    await Bun.sleep(20)
  }
  throw new Error(`Owned process group ${pid} survived teardown`)
}

async function waitReady(
  url: string,
  owner: ReturnType<typeof ownedProcess>,
  log: string[],
  timeout: number,
) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) {
    if (
      owner.error() ||
      owner.child.exitCode !== null ||
      owner.child.signalCode !== null
    )
      throw new Error(
        `Process exited before ${url}: ${owner.error() ?? owner.child.exitCode}\n${log.join('')}`,
      )
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(1_000) })
      if (response.ok) return
    } catch {
      /* Startup is polled only for this run's owned process. */
    }
    await Bun.sleep(100)
  }
  throw new Error(`Timed out waiting for ${url}\n${log.join('')}`)
}

export async function bootServer(
  kind: ServerKind,
  options: { extensionDir?: string; headless?: boolean } = {},
): Promise<BootedServer> {
  const root = await mkdtemp(join(tmpdir(), `server-compat-${kind}-`))
  const processes: ReturnType<typeof ownedProcess>[] = []
  const log: string[] = []
  let stopped = false
  const stop = async () => {
    if (stopped) return
    stopped = true
    cleanups.delete(stop)
    // Stop the server before CDP: TS exits on CDP loss, which would obscure its shutdown status.
    const failures: unknown[] = []
    for (const process of processes.reverse()) {
      try {
        await terminate(process.child, process.exited)
      } catch (error) {
        failures.push(error)
      }
    }
    await writeFile(join(root, 'process.log'), log.join(''))
    if (process.env.SERVER_COMPAT_KEEP_SANDBOX !== '1')
      await rm(root, { recursive: true, force: true })
    else console.log(`Sandbox retained: ${root}`)
    if (failures.length)
      throw new AggregateError(failures, 'Process teardown failed')
  }
  cleanups.add(stop)
  try {
    const serverPort = await freePort()
    let cdpPort = await freePort()
    while (cdpPort === serverPort) cdpPort = await freePort()
    for (const name of [
      'home',
      'state',
      'resources',
      'execution',
      'config',
      'cache',
    ])
      await mkdir(join(root, name))
    await writeFile(
      join(root, 'state/installation.json'),
      JSON.stringify({ install_id: '11111111-1111-4111-8111-111111111111' }),
    )
    // Do not inherit provider/OAuth credentials, owner config roots, NODE_OPTIONS, or a live MCP URL.
    const env: NodeJS.ProcessEnv = {
      PATH: process.env.PATH,
      TMPDIR: process.env.TMPDIR,
      SystemRoot: process.env.SystemRoot,
      HOME: join(root, 'home'),
      USERPROFILE: join(root, 'home'),
      XDG_CONFIG_HOME: join(root, 'config'),
      XDG_CACHE_HOME: join(root, 'cache'),
      BROWSEROS_DIR: join(root, 'state'),
      NODE_ENV: 'development',
      LOG_LEVEL: 'warn',
      BROWSEROS_TRUSTED_ORIGINS: trustedOrigin,
      CARGO_HOME: process.env.CARGO_HOME ?? join(homedir(), '.cargo'),
      RUSTUP_HOME: process.env.RUSTUP_HOME ?? join(homedir(), '.rustup'),
    }
    const config = join(root, 'sidecar.json')
    await writeFile(
      config,
      JSON.stringify({
        ports: { server: serverPort, cdp: cdpPort },
        directories: {
          resources: join(root, 'resources'),
          execution: join(root, 'execution'),
        },
        flags: { allow_remote_in_mcp: false },
        instance: { install_id: '11111111-1111-4111-8111-111111111111' },
      }),
    )
    const browser =
      process.env.BROWSEROS_BINARY ??
      '/Applications/BrowserOS.app/Contents/MacOS/BrowserOS'
    if (!existsSync(browser))
      throw new Error(
        `TS requires live CDP before HTTP startup (recon §4.4); BrowserOS not found at ${browser}. Set BROWSEROS_BINARY to the executable.`,
      )
    const browserArgs = [
      browser,
      ...(options.headless === false ? [] : ['--headless=new']),
      '--enable-automation',
      '--no-first-run',
      '--no-default-browser-check',
      '--use-mock-keychain',
      '--disable-browseros-server',
      '--disable-browseros-extensions',
      '--disable-background-networking',
      '--disable-component-update',
      `--browseros-mcp-port=${serverPort}`,
      ...(options.extensionDir
        ? [`--load-extension=${resolve(options.extensionDir)}`]
        : []),
      `--user-data-dir=${join(root, 'profile')}`,
      `--remote-debugging-port=${cdpPort}`,
      '--remote-debugging-address=127.0.0.1',
      'about:blank',
    ]
    const browserProcess = ownedProcess(browserArgs, env, log)
    processes.push(browserProcess)
    await waitReady(
      `http://127.0.0.1:${cdpPort}/json/version`,
      browserProcess,
      log,
      30_000,
    )
    const command =
      kind === 'ts'
        ? [process.execPath, 'apps/server/src/index.ts', '--config', config]
        : process.env.SERVER_RUST_BINARY
          ? [resolve(process.env.SERVER_RUST_BINARY), '--config', config]
          : ['cargo', 'run', '-p', 'server-rust', '--', '--config', config]
    const server = ownedProcess(command, env, log)
    processes.push(server)
    const url = `http://127.0.0.1:${serverPort}`
    await waitReady(
      `${url}/system/health`,
      server,
      log,
      Number(
        process.env.SERVER_COMPAT_BOOT_TIMEOUT_MS ??
          (kind === 'rust' ? 180_000 : 30_000),
      ),
    )
    return { kind, root, url, cdpPort, stop }
  } catch (error) {
    await stop()
    throw error
  }
}

export function parseKinds(argv = process.argv.slice(2)): {
  a: ServerKind
  b: ServerKind
} {
  const value = (flag: string, fallback: ServerKind) => {
    const index = argv.indexOf(flag)
    const kind = index < 0 ? fallback : argv[index + 1]
    if (kind !== 'ts' && kind !== 'rust')
      throw new Error(`${flag} must be ts or rust`)
    return kind
  }
  return { a: value('--a', 'ts'), b: value('--b', 'rust') }
}

export async function runCli(main: () => Promise<void>) {
  const onSignal = async () => {
    await Promise.allSettled([...cleanups].map((stop) => stop()))
    process.exit(130)
  }
  process.once('SIGINT', onSignal)
  process.once('SIGTERM', onSignal)
  try {
    await main()
  } catch (error) {
    console.error(error)
    process.exitCode = 1
  } finally {
    const results = await Promise.allSettled(
      [...cleanups].map((stop) => stop()),
    )
    for (const result of results)
      if (result.status === 'rejected') {
        console.error(result.reason)
        process.exitCode = 1
      }
    process.removeListener('SIGINT', onSignal)
    process.removeListener('SIGTERM', onSignal)
  }
}

if (import.meta.main)
  await runCli(async () => {
    const server = await bootServer(parseKinds().a)
    console.log(
      `PASS ${server.kind} health ${server.url}; owned CDP ${server.cdpPort}`,
    )
    const health = await fetch(`${server.url}/system/health`).then((response) =>
      response.text(),
    )
    console.log(health)
    await server.stop()
  })
