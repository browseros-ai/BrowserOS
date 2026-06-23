import { existsSync } from 'node:fs'
import { Mutex } from 'async-mutex'
import { CdpBackend } from '../../src/browser/backends/cdp'
import { Browser } from '../../src/browser/browser'
import {
  type BrowserConfig,
  isBrowserRunning,
  killBrowser,
  spawnBrowser,
} from './browser'
import { killProcessOnPort } from './kill-port'
import { createTestRuntimePlan, type TestRuntimePlan } from './test-runtime'

const mutex = new Mutex()
let cachedCdp: CdpBackend | null = null
let cachedBrowser: Browser | null = null
let runtimePlan: TestRuntimePlan | null = null

async function canReuseCachedBrowser(): Promise<boolean> {
  if (!cachedBrowser || !cachedCdp?.isConnected() || !runtimePlan) return false
  if (!(await isBrowserRunning(runtimePlan.ports.cdp))) return false
  try {
    await cachedCdp.Browser.getVersion()
    return true
  } catch {
    return false
  }
}

async function getOrCreateBrowser(): Promise<Browser> {
  if (await canReuseCachedBrowser()) return cachedBrowser as Browser

  if (runtimePlan && !existsSync(runtimePlan.userDataDir)) {
    runtimePlan = null
  }

  if (!runtimePlan) {
    runtimePlan = await createTestRuntimePlan()
  }

  if (runtimePlan.usesFixedPorts) {
    await killProcessOnPort(runtimePlan.ports.cdp)
  }

  const config: BrowserConfig = {
    cdpPort: runtimePlan.ports.cdp,
    serverPort: runtimePlan.ports.server,
    extensionPort: runtimePlan.ports.extension,
    binaryPath: runtimePlan.binaryPath,
    userDataDir: runtimePlan.userDataDir,
    headless: runtimePlan.headless,
    extraArgs: runtimePlan.extraArgs,
  }
  await spawnBrowser(config)

  cachedCdp = new CdpBackend({
    port: runtimePlan.ports.cdp,
    exitOnReconnectFailure: false,
  })
  await cachedCdp.connect()

  cachedBrowser = new Browser(cachedCdp)
  return cachedBrowser
}

/** Tears down the cached browser/CDP pair used by withBrowser tests. */
export async function cleanupWithBrowser(): Promise<void> {
  await mutex.runExclusive(async () => {
    await cachedCdp?.disconnect().catch(() => {})
    await killBrowser()
    cachedCdp = null
    cachedBrowser = null
    runtimePlan = null
  })
}

export interface WithBrowserContext {
  browser: Browser
}

export async function withBrowser(
  cb: (ctx: WithBrowserContext) => Promise<void>,
): Promise<void> {
  return await mutex.runExclusive(async () => {
    const browser = await getOrCreateBrowser()
    await cb({
      browser,
    })
  })
}
