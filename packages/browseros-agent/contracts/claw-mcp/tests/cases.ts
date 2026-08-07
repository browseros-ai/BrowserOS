/**
 * Behavioral contract for the Rust server's `/mcp` surface. Every case drives
 * the production entrypoint over real HTTP with a real BrowserOS attached and
 * asserts observable tool behavior only.
 *
 * CASE ORDER IS LOAD-BEARING: cases run sequentially per server in
 * array order against one shared browser profile. State-poisoning
 * cases (killing the browser) must stay last; cases that open dialogs
 * or pages clean them up before returning.
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
  /** `tabs new` on the given session (default primary); returns the page id and tracks it for post-case cleanup. */
  openPage(url: string, session?: McpSession): Promise<number>
  scratchDir: string
}

export interface ContractCase {
  name: string
  /** ~12 cases carry true: the <60s tier run by test:claw-mcp-smoke. */
  smoke?: boolean
  run(ctx: CaseContext): Promise<void>
}

import { actCases } from './cases-act'
import { captureIoCases } from './cases-capture-io'
import { clawLayerCases } from './cases-claw-layer'
import { navigateSnapshotCases } from './cases-navigate-snapshot'
import { readEvalCases } from './cases-read-eval'
import { snapshotConcurrencyCases } from './cases-snapshot-concurrency'
import { tabsCases } from './cases-tabs'
import { transportCases } from './cases-transport'

// Order is load-bearing: clawLayerCases ends with the browser-kill case,
// which must be the final case per server run — it poisons the browser.
export const contractCases: ContractCase[] = [
  ...transportCases,
  ...tabsCases,
  ...navigateSnapshotCases,
  ...snapshotConcurrencyCases,
  ...actCases,
  ...readEvalCases,
  ...captureIoCases,
  ...clawLayerCases,
]
