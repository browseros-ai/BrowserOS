/**
 * Boots the production Rust server against the harness browser's CDP port via
 * a temporary sidecar. The sandboxed user and BrowserClaw directories keep
 * audit state, downloads, and spill files isolated from the developer's data.
 */

import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { findFreePort } from './browser'

const MONOREPO_ROOT = resolve(import.meta.dir, '../../..')
const HEALTH_ATTEMPTS = 150
const HEALTH_INTERVAL_MS = 200
const STOP_GRACE_MS = 6_000
const LOG_TAIL_LINES = 40

export interface ContractServer {
  baseUrl: string
  /** Where this server writes spill files: `<dir>/tool-output/`. */
  toolOutputDir: string
  /** Sandboxed home — downloads and other per-user paths live under it. */
  homeDir: string
  stop(): Promise<void>
}

interface SpawnedServer {
  child: Bun.Subprocess<'ignore', 'pipe', 'pipe'>
  logTail(): string
}

function watchLogs(
  child: Bun.Subprocess<'ignore', 'pipe', 'pipe'>,
): () => string {
  const lines: string[] = []
  const consume = async (stream: ReadableStream<Uint8Array>) => {
    const decoder = new TextDecoder()
    let pending = ''
    try {
      for await (const chunk of stream) {
        pending += decoder.decode(chunk, { stream: true })
        const parts = pending.split('\n')
        pending = parts.pop() ?? ''
        lines.push(...parts)
        if (lines.length > LOG_TAIL_LINES) {
          lines.splice(0, lines.length - LOG_TAIL_LINES)
        }
      }
      if (pending) lines.push(pending)
    } catch {
      // The stream errors when the process is force-killed mid-read;
      // the log tail is best-effort, so swallow it rather than surface
      // an unhandledRejection.
    }
  }
  void consume(child.stdout)
  void consume(child.stderr)
  return () => lines.join('\n')
}

async function sandboxEnv(root: string): Promise<Record<string, string>> {
  const home = join(root, 'home')
  await mkdir(join(home, '.config'), { recursive: true })
  return {
    ...process.env,
    HOME: home,
    USERPROFILE: home,
    XDG_CONFIG_HOME: join(home, '.config'),
    CLAUDE_CONFIG_DIR: home,
    APPDATA: join(home, 'AppData', 'Roaming'),
    LOCALAPPDATA: join(home, 'AppData', 'Local'),
    BROWSERCLAW_DIR: join(root, 'browserclaw'),
    BROWSEROS_DIR: join(root, 'browseros'),
    CLAW_ANALYTICS_ENABLED: 'false',
  }
}

async function writeSidecar(
  root: string,
  serverPort: number,
  cdpPort: number,
): Promise<string> {
  const resources = join(root, 'resources')
  await mkdir(resources, { recursive: true })
  const path = join(root, 'sidecar.json')
  await writeFile(
    path,
    JSON.stringify({
      ports: { server: serverPort, cdp: cdpPort },
      directories: { resources },
      flags: { devMode: false },
    }),
  )
  return path
}

async function waitUntilHealthy(
  baseUrl: string,
  spawned: SpawnedServer,
  label: string,
): Promise<void> {
  for (let attempt = 0; attempt < HEALTH_ATTEMPTS; attempt += 1) {
    if (spawned.child.exitCode !== null) {
      throw new Error(
        `${label} exited with ${spawned.child.exitCode} before becoming healthy:\n${spawned.logTail()}`,
      )
    }
    try {
      const response = await fetch(`${baseUrl}/system/health`, {
        signal: AbortSignal.timeout(1_000),
      })
      if (response.ok) return
    } catch {}
    await Bun.sleep(HEALTH_INTERVAL_MS)
  }
  spawned.child.kill(9)
  throw new Error(`${label} never became healthy:\n${spawned.logTail()}`)
}

async function stopServer(spawned: SpawnedServer, root: string): Promise<void> {
  const { child } = spawned
  if (child.exitCode === null) {
    child.kill()
    const forceKill = setTimeout(() => child.kill(9), STOP_GRACE_MS)
    await child.exited
    clearTimeout(forceKill)
  }
  await rm(root, { recursive: true, force: true })
}

async function startServer(
  cmd: string[],
  cdpPort: number,
  tmpPrefix: string,
): Promise<ContractServer> {
  const root = await mkdtemp(join(tmpdir(), tmpPrefix))
  const serverPort = await findFreePort()
  const sidecar = await writeSidecar(root, serverPort, cdpPort)
  const child = Bun.spawn({
    cmd: [...cmd, '--config', sidecar],
    cwd: MONOREPO_ROOT,
    env: await sandboxEnv(root),
    stdout: 'pipe',
    stderr: 'pipe',
  }) as Bun.Subprocess<'ignore', 'pipe', 'pipe'>
  const spawned: SpawnedServer = { child, logTail: watchLogs(child) }
  const baseUrl = `http://127.0.0.1:${serverPort}`
  try {
    await waitUntilHealthy(baseUrl, spawned, 'Rust claw server')
  } catch (error) {
    await stopServer(spawned, root)
    throw error
  }
  return {
    baseUrl,
    toolOutputDir: join(root, 'browseros', 'tool-output'),
    homeDir: join(root, 'home'),
    stop: () => stopServer(spawned, root),
  }
}

export const RUST_BINARY = resolve(
  MONOREPO_ROOT,
  'target/debug/browseros-claw-server-rs',
)

/**
 * `run.ts` pre-builds the debug binary so test timeouts never absorb a cold
 * Cargo build.
 */
export function buildRustServer(): void {
  const build = Bun.spawnSync({
    cmd: ['cargo', 'build', '--locked', '-p', 'claw-server-rust'],
    cwd: MONOREPO_ROOT,
    stdout: 'inherit',
    stderr: 'inherit',
  })
  if (build.exitCode !== 0) {
    throw new Error(
      `cargo build -p claw-server-rust failed (${build.exitCode})`,
    )
  }
}

export async function startRustServer(
  cdpPort: number,
): Promise<ContractServer> {
  if (!(await Bun.file(RUST_BINARY).exists())) {
    buildRustServer()
  }
  return await startServer([RUST_BINARY], cdpPort, 'claw-mcp-rust-')
}
