/**
 * Real-BrowserOS runtime for the claw-mcp contract suite, distilled
 * from `apps/server/tests/__helpers__/{browser,test-runtime}.ts` with
 * one deliberate change: there is NO default binary path. The suite is
 * opt-in — it runs only when `BROWSEROS_BINARY` is explicitly set and
 * skips cleanly everywhere else, so `bun run test` stays green on
 * machines and CI runners without a browser build.
 */

import { rmSync } from 'node:fs'
import { mkdtemp } from 'node:fs/promises'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const CDP_POLL_ATTEMPTS = 60
const CDP_POLL_INTERVAL_MS = 500
const EXIT_GRACE_MS = 1_500

export interface BrowserHandle {
  cdpPort: number
  userDataDir: string
  isRunning(): Promise<boolean>
  kill(): Promise<void>
}

export function isSuiteEnabled(): boolean {
  return Boolean(process.env.BROWSEROS_BINARY)
}

export function browserBinary(): string {
  const binary = process.env.BROWSEROS_BINARY
  if (!binary) {
    throw new Error(
      'BROWSEROS_BINARY is not set; the claw-mcp contract suite should have been skipped',
    )
  }
  return binary
}

export async function findFreePort(): Promise<number> {
  return await new Promise((resolvePort, reject) => {
    const probe = createServer()
    probe.once('error', reject)
    probe.listen(0, '127.0.0.1', () => {
      const address = probe.address()
      if (address === null || typeof address === 'string') {
        probe.close(() => reject(new Error('port probe returned no address')))
        return
      }
      const { port } = address
      probe.close(() => resolvePort(port))
    })
  })
}

export async function isBrowserRunning(cdpPort: number): Promise<boolean> {
  try {
    const response = await fetch(`http://127.0.0.1:${cdpPort}/json/version`, {
      signal: AbortSignal.timeout(1_000),
    })
    return response.ok
  } catch {
    return false
  }
}

function launchArgs(cdpPort: number, userDataDir: string): string[] {
  const args = [
    '--no-first-run',
    '--no-default-browser-check',
    '--use-mock-keychain',
    '--show-component-extension-options',
    '--disable-browseros-extensions',
    '--browseros-dock-icon=dev',
    '--enable-logging=stderr',
  ]
  if (process.env.BROWSEROS_TEST_HEADLESS !== 'false') {
    args.push('--headless=new')
  }
  const extra = process.env.BROWSEROS_TEST_EXTRA_ARGS
  if (extra) {
    args.push(...extra.split(' ').filter(Boolean))
  }
  args.push(
    `--user-data-dir=${userDataDir}`,
    `--remote-debugging-port=${cdpPort}`,
    '--disable-browseros-server',
  )
  return args
}

/**
 * Spawns a fresh headless BrowserOS with its own profile and CDP port.
 * Every call is a cold start on purpose: each server pass in the
 * conformance suite gets a browser no other server has touched.
 */
export async function launchBrowser(): Promise<BrowserHandle> {
  const binary = browserBinary()
  const userDataDir = await mkdtemp(
    join(tmpdir(), 'browseros-contract-browser-'),
  )
  const cdpPort = await findFreePort()
  const child = Bun.spawn({
    cmd: [binary, ...launchArgs(cdpPort, userDataDir)],
    stdout: process.env.BROWSEROS_TEST_DEBUG === 'true' ? 'inherit' : 'ignore',
    stderr: process.env.BROWSEROS_TEST_DEBUG === 'true' ? 'inherit' : 'ignore',
    detached: process.platform !== 'win32',
  })

  const signalBrowserTree = (signal: NodeJS.Signals): void => {
    if (process.platform === 'win32') {
      child.kill(signal)
      return
    }
    try {
      // AppImage launchers spawn the actual browser beneath the direct child.
      // The detached process group lets teardown reach that whole tree.
      process.kill(-child.pid, signal)
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error
    }
  }

  const abandon = (message: string): never => {
    signalBrowserTree('SIGKILL')
    rmSync(userDataDir, { recursive: true, force: true })
    throw new Error(message)
  }
  for (let attempt = 0; attempt < CDP_POLL_ATTEMPTS; attempt += 1) {
    if (child.exitCode !== null) {
      abandon(`BrowserOS exited during startup (${child.exitCode})`)
    }
    if (await isBrowserRunning(cdpPort)) break
    if (attempt === CDP_POLL_ATTEMPTS - 1) {
      abandon(`BrowserOS CDP endpoint never came up on ${cdpPort}`)
    }
    await Bun.sleep(CDP_POLL_INTERVAL_MS)
  }

  let killed = false
  return {
    cdpPort,
    userDataDir,
    isRunning: () => isBrowserRunning(cdpPort),
    kill: async () => {
      if (killed) return
      killed = true
      signalBrowserTree('SIGTERM')
      const forceKill = setTimeout(
        () => signalBrowserTree('SIGKILL'),
        EXIT_GRACE_MS,
      )
      await child.exited
      if (await isBrowserRunning(cdpPort)) signalBrowserTree('SIGKILL')
      clearTimeout(forceKill)
      rmSync(userDataDir, { recursive: true, force: true })
    },
  }
}
