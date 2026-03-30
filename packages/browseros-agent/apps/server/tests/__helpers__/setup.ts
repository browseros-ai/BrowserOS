/**
 * @license
 * Copyright 2025 BrowserOS
 *
 * Unified test environment orchestrator.
 * Ensures server + browser + extension are all ready.
 */
import {
  type BrowserConfig,
  getBrowserState,
  killBrowser,
  spawnBrowser,
} from './browser'
import { getServerState, killServer, spawnServer } from './server'
import { createTestRuntimePlan, type TestRuntimePlan } from './test-runtime'
import { killProcessOnPort } from './utils'

export interface TestEnvironmentConfig {
  cdpPort: number
  serverPort: number
  extensionPort: number
}

let runtimePlan: TestRuntimePlan | null = null

function configsMatch(
  a: TestEnvironmentConfig,
  b: TestEnvironmentConfig,
): boolean {
  return (
    a.cdpPort === b.cdpPort &&
    a.serverPort === b.serverPort &&
    a.extensionPort === b.extensionPort
  )
}

/**
 * Ensures the full BrowserOS test environment is ready:
 * 1. Server running and healthy
 * 2. Browser running with CDP available
 *
 * Reuses existing processes if already running with same config.
 */
export async function ensureBrowserOS(
  options?: Partial<TestEnvironmentConfig>,
): Promise<TestEnvironmentConfig> {
  if (!runtimePlan) {
    runtimePlan = await createTestRuntimePlan()
  }

  const config: TestEnvironmentConfig = {
    cdpPort: options?.cdpPort ?? runtimePlan.ports.cdp,
    serverPort: options?.serverPort ?? runtimePlan.ports.server,
    extensionPort: options?.extensionPort ?? runtimePlan.ports.extension,
  }

  // Fast path: already running with same config
  const serverState = getServerState()
  const browserState = getBrowserState()
  if (
    serverState &&
    browserState &&
    configsMatch(serverState.config, config) &&
    configsMatch(browserState.config, config)
  ) {
    console.log('Reusing existing test environment')
    return config
  }

  // Config changed or not running: full setup
  console.log('\n=== Setting up BrowserOS test environment ===')

  // 1. Kill conflicting processes on ports
  await killProcessOnPort(config.serverPort)
  await killProcessOnPort(config.extensionPort)
  await killProcessOnPort(config.cdpPort)

  // 2. Start browser first so CDP is available before server startup.
  const browserConfig: BrowserConfig = {
    ...config,
    binaryPath: runtimePlan.binaryPath,
    userDataDir: runtimePlan.userDataDir,
    headless: runtimePlan.headless,
    extraArgs: runtimePlan.extraArgs,
  }
  await spawnBrowser(browserConfig)

  // 3. Start server once CDP is available.
  await spawnServer(config)

  console.log('=== Test environment ready ===\n')
  return config
}

/**
 * Cleans up the full BrowserOS test environment.
 */
export async function cleanupBrowserOS(): Promise<void> {
  console.log('\n=== Cleaning up BrowserOS test environment ===')
  await killBrowser()
  await killServer()
  runtimePlan = null
  console.log('=== Cleanup complete ===\n')
}
