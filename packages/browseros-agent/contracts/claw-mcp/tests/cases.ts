/**
 * Behavioral contract for the Rust server's `/mcp` surface. Every case drives
 * the production entrypoint over real HTTP with a real BrowserOS attached and
 * asserts observable tool behavior only.
 *
 * Cases run sequentially against one suite-owned browser profile. The runner
 * closes tabs between cases, including tabs opened by scripts that fail.
 */

import type { BrowserHandle } from './browser'
import type { McpSession } from './mcp-client'
import type { ContractServer } from './rust-server'

export const CASE_TIMEOUT_MS = 180_000

export interface CaseContext {
  server: ContractServer
  browser: BrowserHandle
  /** Primary MCP session, shared across cases within one server run. */
  mcp: McpSession
  /** Extra sessions (ownership/naming/audit cases); auto-closed at run end. */
  openSession(clientName?: string): Promise<McpSession>
  /** URL on the primary fixture origin. */
  fixture(path: string): string
  /** URL on the secondary fixture origin (cross-origin iframes). */
  fixture2(path: string): string
  /** `run` opens a tab on the given session and returns its native tab id. */
  openPage(url: string, session?: McpSession): Promise<number>
  scratchDir: string
}

export interface ContractCase {
  name: string
  /** Browser arguments scoped to this case; changing them restarts the isolated runtime. */
  browserArgs?: string[]
  /** Override the suite's launch mode; visual batches can match hosted Neo's headed X server. */
  headless?: boolean
  /** Core browser and transport cases run by test:claw-mcp-smoke. */
  smoke?: boolean
  run(ctx: CaseContext): Promise<void>
}

import { neoRunCases } from './cases-neo-run'
import { playwrightRunCases } from './cases-playwright-run'
import { transportCases } from './cases-transport'

export const contractCases: ContractCase[] = [
  ...transportCases,
  ...neoRunCases,
  ...playwrightRunCases,
]
