import { existsSync } from 'node:fs'
import { Mutex } from 'async-mutex'
import { CdpBackend } from '../../src/browser/backends/cdp'
import { Browser } from '../../src/browser/browser'
import type { ToolDefinition } from '../../src/tools/framework'
import { executeTool } from '../../src/tools/framework'
import type { ToolResult } from '../../src/tools/response'
import { type BrowserConfig, killBrowser, spawnBrowser } from './browser'
import { createTestRuntimePlan, type TestRuntimePlan } from './test-runtime'
import { killProcessOnPort } from './utils'

const mutex = new Mutex()
let cachedCdp: CdpBackend | null = null
let cachedBrowser: Browser | null = null
let runtimePlan: TestRuntimePlan | null = null

async function getOrCreateBrowser(): Promise<Browser> {
  if (cachedBrowser && cachedCdp?.isConnected()) return cachedBrowser

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

  cachedCdp = new CdpBackend({ port: runtimePlan.ports.cdp })
  await cachedCdp.connect()

  cachedBrowser = new Browser(cachedCdp)
  return cachedBrowser
}

export async function cleanupWithBrowser(): Promise<void> {
  await mutex.runExclusive(async () => {
    await killBrowser()
    cachedCdp = null
    cachedBrowser = null
    runtimePlan = null
  })
}

export interface WithBrowserContext {
  browser: Browser
  execute: (tool: ToolDefinition, args: unknown) => Promise<ToolResult>
}

export async function withBrowser(
  cb: (ctx: WithBrowserContext) => Promise<void>,
): Promise<void> {
  return await mutex.runExclusive(async () => {
    const browser = await getOrCreateBrowser()

    const execute = async (
      tool: ToolDefinition,
      args: unknown,
    ): Promise<ToolResult> => {
      const signal = AbortSignal.timeout(30_000)
      return executeTool(
        tool,
        args,
        {
          browser,
          directories: { workingDir: process.cwd() },
        },
        signal,
      )
    }

    await cb({ browser, execute })
  })
}
