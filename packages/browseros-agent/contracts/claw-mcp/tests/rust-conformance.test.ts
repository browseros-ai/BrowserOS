/**
 * Rust MCP conformance suite. It boots a fresh BrowserOS profile, attaches the
 * production Rust server to its CDP port, and runs every behavioral contract
 * case sequentially through a raw MCP session.
 *
 * Gated: without BROWSEROS_BINARY every test is skipped. `CLAW_MCP_SMOKE=1`
 * filters to the smoke tier. Prefer `bun contracts/claw-mcp/tests/run.ts`,
 * which pre-builds the server outside test timeouts.
 */

import { afterAll, describe, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { type FixturePair, startFixturePair } from '../fixtures/server'
import { type BrowserHandle, isSuiteEnabled, launchBrowser } from './browser'
import { runCaptureMode } from './capture'
import { CASE_TIMEOUT_MS, type CaseContext, contractCases } from './cases'
import { expectOk, waitUntil } from './helpers'
import { McpSession, textOf } from './mcp-client'
import { type ContractServer, startRustServer } from './rust-server'

const gate = isSuiteEnabled() ? describe : describe.skip
const onlyCases = process.env.CLAW_MCP_ONLY?.trim()
const activeCases = (
  process.env.CLAW_MCP_SMOKE === '1'
    ? contractCases.filter((contractCase) => contractCase.smoke)
    : contractCases
).filter((contractCase) => !onlyCases || contractCase.name.includes(onlyCases))

interface ServerRun {
  browserArgs: string[]
  headless: boolean
  server: ContractServer
  browser: BrowserHandle
  mcp: McpSession
  extraSessions: McpSession[]
  scratchDir: string
}

let fixtures: FixturePair | undefined
let run: ServerRun | undefined
let captured = false

async function ensureFixtures(): Promise<FixturePair> {
  fixtures ??= await startFixturePair()
  return fixtures
}

async function ensureRun(
  browserArgs: string[] = [],
  headless = process.env.BROWSEROS_TEST_HEADLESS !== 'false',
): Promise<ServerRun> {
  // Density and headed-capture cases need real launch settings, not page emulation.
  // Replace only the suite-owned runtime when a case's launch contract changes;
  // ordinary cases keep their existing arguments and shared-session ordering.
  if (
    run &&
    (JSON.stringify(run.browserArgs) !== JSON.stringify(browserArgs) ||
      run.headless !== headless)
  ) {
    await teardownRun()
  }
  if (run) return run

  await ensureFixtures()
  const browser = await launchBrowser(browserArgs, headless)
  // Everything after the launch is wrapped so a failure (capture-mode,
  // server boot, browser attach, scratch-dir mint) never leaks the
  // browser: nothing is stored in `run` until the run is fully built,
  // so afterAll's teardownRun could not otherwise reclaim it.
  let server: ContractServer | undefined
  try {
    // One-time side quest: with a browser up and no server attached yet,
    // capture-mode dumps raw CDP payloads for the serde fixtures.
    if (!captured && process.env.CLAW_MCP_CAPTURE_DIR) {
      captured = true
      const pair = await ensureFixtures()
      await runCaptureMode(
        browser.cdpPort,
        pair.primary,
        process.env.CLAW_MCP_CAPTURE_DIR,
      )
    }
    server = await startRustServer(browser.cdpPort)
    const mcp = await connectAttached(server, 'claw-contract')
    run = {
      browserArgs,
      headless,
      server,
      browser,
      mcp,
      extraSessions: [],
      scratchDir: await mkdtemp(join(tmpdir(), 'claw-mcp-scratch-')),
    }
    return run
  } catch (error) {
    await server?.stop().catch(() => {})
    await browser.kill().catch(() => {})
    throw error
  }
}

/** An MCP session once the server has attached to the browser. */
async function connectAttached(
  server: ContractServer,
  clientName: string,
): Promise<McpSession> {
  const mcp = await McpSession.connect(server.baseUrl, clientName)
  // Health can precede the SDK's browser attachment. Read actual tabs through
  // run so an unavailable browser cannot look ready just because the realm runs.
  await waitUntil(
    async () => {
      const result = await mcp.callTool('run', {
        code: 'return (await context.pages()).map(p => p.tabId)',
      })
      return !result.isError && Array.isArray(result.structuredContent?.value)
    },
    'Rust server and SDK to attach to the browser',
    { timeoutMs: 30_000, intervalMs: 500 },
  )
  return mcp
}

function makeContext(run: ServerRun): CaseContext {
  const pair = fixtures
  if (!pair) throw new Error('fixture servers not started')
  return {
    server: run.server,
    browser: run.browser,
    mcp: run.mcp,
    scratchDir: run.scratchDir,
    async openSession(clientName = 'claw-contract-extra') {
      const session = await McpSession.connect(run.server.baseUrl, clientName)
      run.extraSessions.push(session)
      return session
    },
    fixture: (path) => pair.primary.url(path),
    fixture2: (path) => pair.secondary.url(path),
    async openPage(url, session = run.mcp) {
      const result = await session.callTool('run', {
        code: `const p = await context.newPage(); await p.goto(${JSON.stringify(url)}); return p.tabId`,
      })
      expectOk(result, 'open fixture tab')
      const tabId = result.structuredContent?.value
      if (typeof tabId !== 'number') {
        throw new Error(`run did not return a tab id: ${textOf(result)}`)
      }
      return tabId
    },
  }
}

/**
 * This profile belongs entirely to the suite. Enumerating it also reclaims tabs
 * created before a script throws, which cannot be tracked from its return value.
 */
async function cleanupCase(run: ServerRun): Promise<void> {
  if (!(await run.browser.isRunning())) return
  expectOk(
    await run.mcp.callTool('run', {
      code: 'for (const p of await context.pages()) await p.close(); return null',
    }),
    'close case tabs',
  )
}

async function teardownRun(): Promise<void> {
  if (!run) return
  const current = run
  run = undefined
  for (const session of [...current.extraSessions, current.mcp]) {
    await session.close().catch(() => {})
  }
  await current.server.stop().catch(() => {})
  await current.browser.kill().catch(() => {})
  await rm(current.scratchDir, { recursive: true, force: true })
}

gate('Rust /mcp conformance', () => {
  // The server's shutdown grace plus profile cleanup exceeds Bun's default hook limit.
  afterAll(async () => {
    await teardownRun()
  }, 30_000)

  for (const contractCase of activeCases) {
    test(
      contractCase.name,
      async () => {
        const activeRun = await ensureRun(
          contractCase.browserArgs,
          contractCase.headless,
        )
        try {
          await contractCase.run(makeContext(activeRun))
        } finally {
          await cleanupCase(activeRun)
        }
      },
      CASE_TIMEOUT_MS,
    )
  }
})

afterAll(async () => {
  await fixtures?.stop()
})
