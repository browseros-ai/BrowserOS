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
import { parsePageId, waitUntil } from './helpers'
import { McpSession, textOf } from './mcp-client'
import { type ContractServer, startRustServer } from './rust-server'

const gate = isSuiteEnabled() ? describe : describe.skip
const activeCases =
  process.env.CLAW_MCP_SMOKE === '1'
    ? contractCases.filter((contractCase) => contractCase.smoke)
    : contractCases

interface ServerRun {
  server: ContractServer
  browser: BrowserHandle
  mcp: McpSession
  extraSessions: McpSession[]
  openedPages: Array<{ session: McpSession; page: number }>
  scratchDir: string
}

let fixtures: FixturePair | undefined
let run: ServerRun | undefined
let captured = false

async function ensureFixtures(): Promise<FixturePair> {
  fixtures ??= await startFixturePair()
  return fixtures
}

async function ensureRun(): Promise<ServerRun> {
  if (run) return run

  await ensureFixtures()
  const browser = await launchBrowser()
  // Everything after the launch is wrapped so a failure (capture-mode,
  // server boot, browser attach, scratch-dir mint) never leaks the
  // browser: nothing is stored in `runs` until the run is fully built,
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
    const mcp = await McpSession.connect(server.baseUrl, 'claw-contract')
    // The rust server attaches to the browser asynchronously after
    // /system/health turns ok; wait until tool calls stop reporting a
    // disconnected browser before running cases.
    await waitUntil(
      async () => {
        const result = await mcp.callTool('tabs', { action: 'list' })
        return !(
          result.isError &&
          textOf(result).includes('browser session not connected')
        )
      },
      'Rust server to attach to the browser',
      { timeoutMs: 30_000, intervalMs: 500 },
    )
    run = {
      server,
      browser,
      mcp,
      extraSessions: [],
      openedPages: [],
      scratchDir: await mkdtemp(join(tmpdir(), 'claw-mcp-scratch-')),
    }
    return run
  } catch (error) {
    await server?.stop().catch(() => {})
    await browser.kill().catch(() => {})
    throw error
  }
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
      const result = await session.callTool('tabs', {
        action: 'new',
        url,
        background: false,
      })
      if (result.isError) {
        throw new Error(`tabs new failed: ${textOf(result)}`)
      }
      const page = parsePageId(result)
      run.openedPages.push({ session, page })
      return page
    },
  }
}

/** Close pages a case opened so the next case starts from a clean browser. */
async function cleanupCase(run: ServerRun): Promise<void> {
  const opened = run.openedPages.splice(0)
  if (!(await run.browser.isRunning())) return
  for (const { session, page } of opened) {
    await session.callTool('tabs', { action: 'close', page }).catch(() => {})
  }
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
  afterAll(async () => {
    await teardownRun()
  })

  for (const contractCase of activeCases) {
    test(
      contractCase.name,
      async () => {
        const activeRun = await ensureRun()
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
