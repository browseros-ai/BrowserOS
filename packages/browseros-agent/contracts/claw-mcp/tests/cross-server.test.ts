/**
 * Cross-server real-browser MCP contract suite. For each server
 * (typescript first, then rust) it boots a FRESH BrowserOS profile,
 * attaches the server to its CDP port, and runs every contract case
 * sequentially through a raw MCP session; a final parity gate compares
 * the semantic signatures both passes recorded and fails on any
 * difference not registered in divergences.ts.
 *
 * Gated: without BROWSEROS_BINARY every describe registers skipped and
 * the file is green anywhere. `CLAW_MCP_SMOKE=1` filters to the smoke
 * tier. Prefer `bun contracts/claw-mcp/tests/run.ts` which pre-builds
 * the rust server outside test timeouts.
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
import { assertParity, comparedKeyCount, recordSignature } from './parity'
import {
  type ContractServer,
  type ServerName,
  startContractServer,
} from './server-adapters'

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
const runs = new Map<ServerName, ServerRun>()
let captured = false

async function ensureFixtures(): Promise<FixturePair> {
  fixtures ??= await startFixturePair()
  return fixtures
}

async function ensureRun(name: ServerName): Promise<ServerRun> {
  const existing = runs.get(name)
  if (existing) return existing

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
    server = await startContractServer(name, browser.cdpPort)
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
      `${name} server to attach to the browser`,
      { timeoutMs: 30_000, intervalMs: 500 },
    )
    const run: ServerRun = {
      server,
      browser,
      mcp,
      extraSessions: [],
      openedPages: [],
      scratchDir: await mkdtemp(join(tmpdir(), 'claw-mcp-scratch-')),
    }
    runs.set(name, run)
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
    record(key, value, options) {
      recordSignature(key, run.server.name, value, options)
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

async function teardownRun(name: ServerName): Promise<void> {
  const run = runs.get(name)
  if (!run) return
  runs.delete(name)
  for (const session of [...run.extraSessions, run.mcp]) {
    await session.close().catch(() => {})
  }
  await run.server.stop().catch(() => {})
  await run.browser.kill().catch(() => {})
  await rm(run.scratchDir, { recursive: true, force: true })
}

for (const name of ['typescript', 'rust'] as const) {
  gate(`${name} /mcp contract`, () => {
    afterAll(async () => {
      await teardownRun(name)
    })

    for (const contractCase of activeCases) {
      test(
        contractCase.name,
        async () => {
          const run = await ensureRun(name)
          try {
            await contractCase.run(makeContext(run))
          } finally {
            await cleanupCase(run)
          }
        },
        CASE_TIMEOUT_MS,
      )
    }
  })
}

gate('cross-server parity', () => {
  test('no unregistered divergences between the servers', () => {
    if (comparedKeyCount() === 0) {
      throw new Error(
        'parity gate ran but no signatures were recorded by both servers',
      )
    }
    assertParity()
  })
})

afterAll(async () => {
  await fixtures?.stop()
})
