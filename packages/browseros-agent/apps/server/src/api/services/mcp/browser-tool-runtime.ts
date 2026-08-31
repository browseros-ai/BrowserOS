/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { BrowserSession } from '@browseros/browser-core/core/session'
import { createBrowserMcpServer } from '@browseros/browser-mcp/mcp-server'
import type { BrowserOutputFileAccess } from '@browseros/browser-mcp/output-file'
import { withBrowserOutputFileAccess } from '@browseros/browser-mcp/output-file'
import { BROWSER_TOOLS } from '@browseros/browser-mcp/registry'
import {
  errorResult,
  executeTool,
  type ToolContext,
  type ToolDefinition,
  type ToolResult,
} from '@browseros/browser-mcp/tools/framework'
import { tabs } from '@browseros/browser-mcp/tools/tabs'
import type { BrowserContext } from '@browseros/shared/schemas/browser-context'
import { z } from 'zod/v4'
import { logger } from '../../../lib/logger'
import { metrics } from '../../../lib/metrics'

export { BROWSEROS_TOOL_LEASE_HEADER } from '../../../lib/browser-tool-lease'

import { shouldLogToolRegistration } from '../../../tools/registration-log-sampling'
import type { KlavisService } from '../klavis'
import type { ServerActivity } from '../server-activity'
import { MCP_INSTRUCTIONS } from './mcp-prompt'

export interface BrowserTabTouchedEvent {
  conversationId: string
  runId?: string
  tabId: number
}

export interface BrowserToolRuntimeDeps {
  version: string
  browserSession: BrowserSession
  klavis?: KlavisService
  activity?: ServerActivity
  onTabTouched?: (event: BrowserTabTouchedEvent) => Promise<void> | void
}

export interface BrowserToolLeaseInput {
  conversationId: string
  readOnly: boolean
  outputFileAccess: BrowserOutputFileAccess
  browserContext?: BrowserContext
  defaultTabGroupId?: string
  source?: string
}

/**
 * A capability held by one conversation. The opaque token crosses loopback
 * HTTP; mutable run identity, browser context, and output grants stay server-side.
 */
export interface BrowserToolLease {
  readonly token: string
  updateBrowserContext(browserContext: BrowserContext | undefined): void
  setActiveRun(runId: string | undefined): void
  revoke(): void
}

export interface CreateBrowserMcpServerInput {
  leaseToken?: string
  requestedReadOnly?: boolean
  includeStructuredContent?: boolean
}

interface LeaseRecord extends BrowserToolLeaseInput {
  token: string
  activeRunId?: string
}

const readOnlyTabs: ToolDefinition = {
  ...tabs,
  description:
    'Inspect browser tabs. `list` returns every open page and `active` returns the front page. Read-only mode cannot open or close tabs.',
  input: z
    .object({
      action: z.enum(['list', 'active']).default('list'),
    })
    .strict(),
  annotations: {
    ...tabs.annotations,
    readOnlyHint: true,
    destructiveHint: false,
  },
}

const READ_ONLY_BROWSER_TOOLS: readonly ToolDefinition[] = [
  readOnlyTabs,
  ...BROWSER_TOOLS.filter(
    (tool) => tool.name !== 'tabs' && tool.annotations?.readOnlyHint,
  ),
]
const BROWSER_TOOL_TIMEOUT_MS = 120_000

export class InvalidBrowserToolLeaseError extends Error {
  constructor() {
    super('Invalid or expired BrowserOS tool lease')
    this.name = 'InvalidBrowserToolLeaseError'
  }
}

/**
 * Owns browser-tool leases, authoritative guards, execution, and effects.
 * `/mcp` is the protocol adapter; conversations never execute browser tools
 * through a second in-process path.
 */
export class BrowserToolRuntime {
  private readonly leases = new Map<string, LeaseRecord>()

  constructor(private readonly deps: BrowserToolRuntimeDeps) {}

  createLease(input: BrowserToolLeaseInput): BrowserToolLease {
    const record: LeaseRecord = {
      ...input,
      token: crypto.randomUUID(),
    }
    this.leases.set(record.token, record)

    return {
      token: record.token,
      updateBrowserContext: (browserContext) => {
        const current = this.leases.get(record.token)
        if (current === record) current.browserContext = browserContext
      },
      setActiveRun: (runId) => {
        const current = this.leases.get(record.token)
        if (current === record) current.activeRunId = runId
      },
      revoke: () => {
        if (this.leases.get(record.token) === record) {
          this.leases.delete(record.token)
        }
      },
    }
  }

  hasLease(token: string): boolean {
    return this.leases.has(token)
  }

  createMcpServer(input: CreateBrowserMcpServerInput = {}) {
    const lease = input.leaseToken
      ? this.leases.get(input.leaseToken)
      : undefined
    if (input.leaseToken && !lease) throw new InvalidBrowserToolLeaseError()

    // A query parameter may further restrict a caller, but can never weaken a
    // conversation capability minted as read-only.
    const readOnly = Boolean(lease?.readOnly || input.requestedReadOnly)
    const tools = readOnly ? READ_ONLY_BROWSER_TOOLS : BROWSER_TOOLS
    const source = lease?.source ?? 'mcp'
    const selectedServerNames = lease?.browserContext?.enabledMcpServers ?? []

    const server = createBrowserMcpServer({
      name: 'browseros_mcp',
      title: 'BrowserOS MCP server',
      version: this.deps.version,
      browserSession: this.deps.browserSession,
      defaultWindowId: lease?.browserContext?.windowId,
      defaultTabGroupId: lease?.defaultTabGroupId,
      instructions: MCP_INSTRUCTIONS,
      registration: {
        tools,
        includeStructuredContent: input.includeStructuredContent ?? false,
        // A lease already supplies stable caller identity. Anonymous external
        // clients retain the MCP session handle compatibility field.
        sessionIdentity: !lease,
        executor: (tool, args, context) =>
          this.execute(lease, readOnly, tool, args, context),
        logger,
        onToolExecutionStart: () => this.deps.activity?.beginMcpToolExecution(),
        onToolExecutionEnd: () => this.deps.activity?.endMcpToolExecution(),
        onToolExecuted: (event) => metrics.log('tool_executed', event),
        shouldLogToolRegistration,
        source,
      },
    })

    this.deps.klavis?.registerMcpTools(
      server,
      { selectedServerNames },
      {
        // Managed connectors share the loopback transport but do not use the
        // browser executor below, so carry the lease gate across that seam.
        authorizeCall: lease
          ? () => this.getLeaseCallRejection(lease)
          : undefined,
      },
    )
    return server
  }

  private async execute(
    lease: LeaseRecord | undefined,
    readOnly: boolean,
    tool: ToolDefinition,
    args: Record<string, unknown>,
    context: ToolContext,
  ): Promise<ToolResult> {
    // Server construction validates the token once, but streamable MCP clients
    // can outlive their conversation. Revalidate every call so revocation cuts
    // off an already-connected transport instead of only blocking reconnects.
    const leaseRejection = lease && this.getLeaseCallRejection(lease)
    if (leaseRejection) return errorResult(leaseRejection)

    const authorizedRunId = lease?.activeRunId

    const rejected = guardReadOnlyCall(readOnly, tool, args)
    if (rejected) return rejected

    const touchedPageIds = new Set<number>()
    const shouldTrackPages = Boolean(lease && this.deps.onTabTouched)
    // The runtime, not a particular MCP client, owns the execution ceiling so
    // native chat, ACP, and external read-only clients share cancellation rules.
    const timeoutSignal = AbortSignal.timeout(BROWSER_TOOL_TIMEOUT_MS)
    const signal = context.signal
      ? AbortSignal.any([context.signal, timeoutSignal])
      : timeoutSignal
    const trackedContext = shouldTrackPages
      ? {
          ...context,
          signal,
          session: trackBrowserSessionPages(context.session, (pageId) =>
            touchedPageIds.add(pageId),
          ),
        }
      : { ...context, signal }
    const result = await withBrowserOutputFileAccess(
      lease?.outputFileAccess,
      () => executeTool(tool, args, trackedContext),
    )
    if (lease && authorizedRunId) {
      for (const pageId of affectedPageIds(tool, args, result)) {
        touchedPageIds.add(pageId)
      }
      await this.applyTabPresenceEffects(lease, authorizedRunId, touchedPageIds)
    }
    return result
  }

  private getLeaseCallRejection(lease: LeaseRecord): string | undefined {
    if (this.leases.get(lease.token) !== lease) {
      return 'Invalid or expired BrowserOS tool lease.'
    }
    // Internal clients are long-lived across turns, but their authority is not.
    // Keep tools/list available for agent construction while rejecting every
    // browser or managed-connector call outside a server-owned run.
    if (!lease.activeRunId) {
      return 'MCP tools require an active conversation run.'
    }
    return undefined
  }

  private async applyTabPresenceEffects(
    lease: LeaseRecord,
    runId: string,
    pageIds: ReadonlySet<number>,
  ): Promise<void> {
    if (!this.deps.onTabTouched) return
    for (const pageId of pageIds) {
      const tabId = this.deps.browserSession.pages.getTabId(pageId)
      if (tabId === undefined) continue
      await this.deps.onTabTouched({
        conversationId: lease.conversationId,
        // Effects carry the run that authorized execution. ConversationPresence
        // discards this event if that run has since ended or been replaced.
        runId,
        tabId,
      })
    }
  }
}

function guardReadOnlyCall(
  readOnly: boolean,
  tool: ToolDefinition,
  args: Record<string, unknown>,
): ToolResult | null {
  if (!readOnly) return null
  if (tool.name !== 'tabs') {
    return tool.annotations?.readOnlyHint
      ? null
      : errorResult(`${tool.name}: unavailable in read-only mode.`)
  }
  const action = typeof args.action === 'string' ? args.action : 'list'
  return action === 'list' || action === 'active'
    ? null
    : errorResult(
        'tabs: read-only mode only supports action="list" or "active".',
      )
}

function affectedPageIds(
  tool: ToolDefinition,
  args: Record<string, unknown>,
  result: ToolResult,
): number[] {
  const action = typeof args.action === 'string' ? args.action : undefined
  // Listing is observation of browser-wide metadata, not interaction with all
  // returned pages. Associating every listed tab would make presence noisy and
  // force-open unrelated panels.
  if (action === 'list') return []

  const pageIds = new Set<number>()
  if (typeof args.page === 'number') pageIds.add(args.page)
  if (Array.isArray(args.pages)) {
    for (const page of args.pages) {
      if (typeof page === 'number') pageIds.add(page)
    }
  }

  if (tool.name === 'tabs' && (action === 'active' || action === 'new')) {
    const structured = asRecord(result.structuredContent)
    const page = structured?.page
    if (typeof page === 'number') pageIds.add(page)
    const pageRecord = asRecord(page)
    if (typeof pageRecord?.pageId === 'number') pageIds.add(pageRecord.pageId)
  }

  return [...pageIds]
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined
}

const SESSION_PAGE_METHODS = new Set([
  'observe',
  'input',
  'nav',
  'screenshot',
  'screenshotForTarget',
  'cdpJsonForPage',
])

/**
 * Wraps one tool call's BrowserSession so even the open-ended `run` tool emits
 * page effects. The proxy is request-local: it does not mutate the shared
 * BrowserSession or leak observations between concurrent conversations.
 */
function trackBrowserSessionPages(
  session: BrowserSession,
  touch: (pageId: number) => void,
): BrowserSession {
  const trackedPages = new Proxy(session.pages, {
    get(target, property) {
      const value = Reflect.get(target, property, target)
      if (typeof value !== 'function') return value

      if (property === 'newPage') {
        return async (...args: unknown[]) => {
          const pageId = await Reflect.apply(value, target, args)
          if (typeof pageId === 'number') touch(pageId)
          return pageId
        }
      }
      if (property === 'getSession' || property === 'getInfo') {
        return (...args: unknown[]) => {
          if (typeof args[0] === 'number') touch(args[0])
          return Reflect.apply(value, target, args)
        }
      }
      return value.bind(target)
    },
  })

  return new Proxy(session, {
    get(target, property) {
      if (property === 'pages') return trackedPages
      const value = Reflect.get(target, property, target)
      if (typeof value !== 'function') return value
      if (typeof property === 'string' && SESSION_PAGE_METHODS.has(property)) {
        return (...args: unknown[]) => {
          if (typeof args[0] === 'number') touch(args[0])
          return Reflect.apply(value, target, args)
        }
      }
      return value.bind(target)
    },
  })
}
