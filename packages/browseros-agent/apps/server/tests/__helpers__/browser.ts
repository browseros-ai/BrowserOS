/**
 * @license
 * Copyright 2025 BrowserOS
 *
 * Low-level BrowserOS process management.
 * Use setup.ts:ensureBrowserOS() for the full test environment.
 */
import type { ChildProcess } from 'node:child_process'
import { spawn, spawnSync } from 'node:child_process'
import { rmSync } from 'node:fs'

const TEST_USER_DATA_PREFIX = 'browseros-test-'
// Keep teardown below Bun's default 5s hook timeout.
const BROWSER_EXIT_GRACE_MS = 1_500
const BROWSER_FORCED_EXIT_MS = 1_000

export interface BrowserConfig {
  cdpPort: number
  serverPort: number
  extensionPort: number
  binaryPath: string
  userDataDir: string
  headless: boolean
  extraArgs: string[]
}

export interface BrowserState {
  process: ChildProcess
  userDataDir: string
  config: BrowserConfig
}

let browserState: BrowserState | null = null

function shouldLogBrowserOutput(): boolean {
  return (
    process.env.CI === 'true' || process.env.BROWSEROS_TEST_DEBUG === 'true'
  )
}

export async function isBrowserRunning(cdpPort: number): Promise<boolean> {
  try {
    const response = await fetch(`http://127.0.0.1:${cdpPort}/json/version`, {
      signal: AbortSignal.timeout(1000),
    })
    return response.ok
  } catch {
    return false
  }
}

async function waitForCdp(cdpPort: number, maxAttempts = 30): Promise<void> {
  for (let i = 0; i < maxAttempts; i++) {
    if (await isBrowserRunning(cdpPort)) {
      return
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  throw new Error(`CDP failed to start on port ${cdpPort} within timeout`)
}

export function getBrowserState(): BrowserState | null {
  return browserState
}

function killOrphanedTestBrowsers(
  message = 'Killed orphaned test browsers from a previous run',
): void {
  // Matches only BrowserOS processes launched with a test user-data-dir
  // (e.g., /var/folders/.../browseros-test-XXXX). Never matches a dev
  // BrowserOS run from ~/Library/Application Support/BrowserOS.
  const result = spawnSync('pkill', ['-9', '-f', TEST_USER_DATA_PREFIX])
  if (result.status === 0) {
    console.log(message)
  }
}

export async function spawnBrowser(
  config: BrowserConfig,
): Promise<BrowserState> {
  if (browserState && browserState.config.cdpPort === config.cdpPort) {
    if (await isBrowserRunning(config.cdpPort)) {
      console.log(`Reusing existing browser on CDP port ${config.cdpPort}`)
      return browserState
    }
  }

  if (browserState) {
    console.log('Config changed, cleaning up existing browser...')
    await killBrowser()
  }

  killOrphanedTestBrowsers()

  console.log(`Starting BrowserOS on CDP port ${config.cdpPort}...`)
  const browserProcess = spawn(
    config.binaryPath,
    [
      '--no-first-run',
      '--no-default-browser-check',
      '--use-mock-keychain',
      '--show-component-extension-options',
      // Match the supported dev/eval launch path and keep legacy BrowserOS
      // extensions from trying to talk to the removed controller bridge.
      '--disable-browseros-extensions',
      '--browseros-dock-icon=dev',
      '--enable-logging=stderr',
      ...(config.headless ? ['--headless=new'] : []),
      ...config.extraArgs,
      `--user-data-dir=${config.userDataDir}`,
      // BrowserOS tests still need Chromium's remote debugging flag here.
      `--remote-debugging-port=${config.cdpPort}`,
      `--browseros-mcp-port=${config.serverPort}`,
      `--browseros-extension-port=${config.extensionPort}`,
      '--disable-browseros-server',
    ],
    {
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  )

  browserProcess.stdout?.on('data', (data) => {
    if (!shouldLogBrowserOutput()) {
      return
    }
    console.log(`[BROWSER] ${data.toString().trim()}`)
  })

  browserProcess.stderr?.on('data', (data) => {
    if (!shouldLogBrowserOutput()) {
      return
    }
    console.error(`[BROWSER] ${data.toString().trim()}`)
  })

  browserProcess.on('error', (error) => {
    console.error('Failed to start BrowserOS:', error)
  })

  console.log('Waiting for CDP to be ready...')
  await waitForCdp(config.cdpPort)
  console.log('CDP is ready')

  browserState = {
    process: browserProcess,
    userDataDir: config.userDataDir,
    config,
  }
  return browserState
}

/** Stops the shared BrowserOS test process and removes its temp profile. */
export async function killBrowser(): Promise<void> {
  const state = browserState
  if (!state) {
    return
  }

  console.log('Shutting down BrowserOS...')
  state.process.kill('SIGTERM')

  await new Promise<void>((resolve) => {
    let settled = false
    let timeout: ReturnType<typeof setTimeout> | undefined
    let forcedTimeout: ReturnType<typeof setTimeout> | undefined

    const finish = () => {
      if (settled) {
        return
      }

      settled = true
      if (timeout) clearTimeout(timeout)
      if (forcedTimeout) clearTimeout(forcedTimeout)
      state.process.off('exit', finish)
      resolve()
    }

    timeout = setTimeout(() => {
      state.process.kill('SIGKILL')
      forcedTimeout = setTimeout(finish, BROWSER_FORCED_EXIT_MS)
    }, BROWSER_EXIT_GRACE_MS)

    state.process.once('exit', finish)
    if (state.process.exitCode !== null || state.process.signalCode !== null) {
      finish()
    }
  })

  console.log('BrowserOS stopped')
  killOrphanedTestBrowsers('Killed dangling BrowserOS test processes')

  if (state.userDataDir) {
    console.log(`Cleaning up temp profile: ${state.userDataDir}`)
    try {
      rmSync(state.userDataDir, { recursive: true, force: true })
    } catch (error) {
      console.error('Failed to clean up temp directory:', error)
    }
  }

  browserState = null
}
