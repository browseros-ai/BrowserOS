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
import {
  CASE_TIMEOUT_MS,
  type CaseContext,
  type ContractCase,
  contractCases,
} from './cases'
import { parsePageId, waitUntil } from './helpers'
import { McpSession, textOf } from './mcp-client'
import { playwrightRunner } from './playwright/runner'
import { type ContractServer, startRustServer } from './rust-server'

const gate = isSuiteEnabled() ? describe : describe.skip
type SharedCaseContext = Omit<CaseContext, 'scopedMcp'>

/** Binds each case registry to the page IDs and session setup its tools use. */
interface ContractRunner<Context extends SharedCaseContext> {
  name: string
  cases: Array<
    Omit<ContractCase, 'run'> & { run: (context: Context) => Promise<void> }
  >
  connectAttached(
    server: ContractServer,
    clientName: string,
  ): Promise<McpSession>
  openPage(run: ServerRun, url: string, session?: McpSession): Promise<number>
  cleanupCase(run: ServerRun): Promise<void>
  caseContext(context: SharedCaseContext, run: ServerRun): Context
}

interface ServerRun {
  browserArgs: string[]
  headless: boolean
  server: ContractServer
  browser: BrowserHandle
  mcp: McpSession
  scoped?: Promise<{ server: ContractServer; mcp: McpSession } | undefined>
  extraSessions: McpSession[]
  openedPages: Array<{ session: McpSession; page: number }>
  scratchDir: string
}

// The lowest BrowserOS release with profile targeting; asserting it gives the scoped server
// the same bound views a BrowserOS-launched server uses.
const PROFILE_TARGETING_VERSION = '0.51.1.3'

let fixtures: FixturePair | undefined
let run: ServerRun | undefined
let captured = false

async function ensureFixtures(): Promise<FixturePair> {
  fixtures ??= await startFixturePair()
  return fixtures
}

async function ensureRun(
  runner: Pick<ContractRunner<SharedCaseContext>, 'connectAttached'>,
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
    const mcp = await runner.connectAttached(server, 'claw-contract')
    run = {
      browserArgs,
      headless,
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

async function connectAttached(
  server: ContractServer,
  clientName: string,
): Promise<McpSession> {
  const mcp = await McpSession.connect(server.baseUrl, clientName)
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
  return mcp
}

/** The scoped server, or `undefined` when the browser has no native profile targeting. */
async function startScoped(
  run: ServerRun,
): Promise<{ server: ContractServer; mcp: McpSession } | undefined> {
  const server = await startRustServer(run.browser.cdpPort, {
    env: { CLAW_NATIVE_BROWSEROS_VERSION: PROFILE_TARGETING_VERSION },
  })
  try {
    const mcp = await connectAttached(server, 'claw-contract-scoped')
    const probe = await mcp.callTool('tabs', { action: 'list' })
    if (
      probe.isError &&
      textOf(probe).includes('target_scope_unsupported_native')
    ) {
      // A BrowserOS without profile targeting runs its server unbound as well.
      console.warn(
        'scoped server unavailable: this browser has no native profile targeting; scoped cases use the primary server',
      )
      await mcp.close().catch(() => {})
      await server.stop().catch(() => {})
      return undefined
    }
    return { server, mcp }
  } catch (error) {
    await server.stop().catch(() => {})
    throw error
  }
}

function makeContext(
  run: ServerRun,
  runner: Pick<ContractRunner<SharedCaseContext>, 'openPage'>,
): SharedCaseContext {
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
    openPage: (url, session) => runner.openPage(run, url, session),
  }
}

/** Close each tracked page through the MCP session that owns it. */
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
  const scoped = await current.scoped?.catch(() => undefined)
  await scoped?.mcp.close().catch(() => {})
  await scoped?.server.stop().catch(() => {})
  await current.server.stop().catch(() => {})
  await current.browser.kill().catch(() => {})
  await rm(current.scratchDir, { recursive: true, force: true })
}

const catalogRunner: ContractRunner<CaseContext> = {
  name: 'catalog',
  cases: contractCases,
  connectAttached,
  async openPage(run, url, session = run.mcp) {
    const result = await session.callTool('tabs', { action: 'new', url })
    if (result.isError) {
      throw new Error(`tabs new failed: ${textOf(result)}`)
    }
    const page = parsePageId(result)
    run.openedPages.push({ session, page })
    return page
  },
  cleanupCase,
  caseContext(context, run) {
    return {
      ...context,
      async scopedMcp() {
        run.scoped ??= startScoped(run)
        return (await run.scoped)?.mcp ?? run.mcp
      },
    }
  },
}

function registerCases<Context extends SharedCaseContext>(
  runner: ContractRunner<Context>,
): void {
  const onlyCases = process.env.CLAW_MCP_ONLY?.trim()
  const smoke = process.env.CLAW_MCP_SMOKE === '1'
  const activeCases = runner.cases.filter(
    (contractCase) =>
      (!smoke || contractCase.smoke) &&
      (!onlyCases || contractCase.name.includes(onlyCases)),
  )

  gate(`Rust /mcp conformance (${runner.name})`, () => {
    // The two catalog servers can each take six seconds to shut down, plus
    // browser and profile cleanup. Bun's default hook timeout is five seconds.
    afterAll(async () => {
      await teardownRun()
    }, 30_000)

    for (const contractCase of activeCases) {
      test(
        contractCase.name,
        async () => {
          const activeRun = await ensureRun(
            runner,
            contractCase.browserArgs,
            contractCase.headless,
          )
          try {
            await contractCase.run(
              runner.caseContext(makeContext(activeRun, runner), activeRun),
            )
          } finally {
            await runner.cleanupCase(activeRun)
          }
        },
        CASE_TIMEOUT_MS,
      )
    }
  })
}

if (process.env.CLAW_PLAYWRIGHT_RUN === '1') {
  registerCases(playwrightRunner)
} else {
  registerCases(catalogRunner)
}

afterAll(async () => {
  await fixtures?.stop()
})
