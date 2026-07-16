/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Dispatches each browser tool through ordered guards, unchanged cancellation
 * composition, and ordered effects whose failures never fail the tool call.
 */

import type { BrowserSession } from '@browseros/browser-core/core/session'
import { BROWSER_TOOLS } from '@browseros/browser-mcp/registry'
import {
  errorResult,
  executeTool,
  type ToolDefinition,
  textResult,
} from '@browseros/browser-mcp/tools/framework'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { type ZodRawShape, z } from 'zod'
import type { AgentKey } from '../domain/agent-key'
import { ownershipStore } from '../domain/ownership'
import { getBrowserSession } from '../lib/browser-session'
import { logger } from '../lib/logger'
import {
  agentIdentityFromClient,
  agentKeyFromClient,
  buildSessionGroupTitle,
  type ClientIdentity,
  clientPrefixFromSlug,
  identityService,
  normalizeSmallName,
} from '../lib/mcp-session'
import {
  CANCELLATION_REASON,
  dispatchCancellation,
} from '../services/dispatch-cancellation'
import { cancellationErrorResult } from './cancellation-result'
import { composeAbortSignals, dispatchErrorText } from './dispatch-util'
import { applyAudit } from './effects/audit'
import { applyOwnershipClaims } from './effects/ownership-claims'
import { applySessionNaming } from './effects/session-naming'
import { applyTabActivity } from './effects/tab-activity'
import { applyAgentTabGroupTitle, applyTabGroups } from './effects/tab-groups'
import { applyTabsListView } from './effects/tabs-list-view'
import { guardBrowserConnected } from './guards/browser-connected'
import { guardNavigateScheme } from './guards/navigate-scheme'
import { guardPageOwnership } from './guards/page-ownership'
import { asRegister, type ToolResult } from './register-fn'

const ARBITRARY_SCRIPT_TOOLS = new Set(['run', 'evaluate'])

export interface ToolCall {
  tool: ToolDefinition
  args: unknown
  sessionId: string
  identity: ClientIdentity | null
  key: AgentKey | null
  agent: { agentId: string; slug: string } | null
  agentLabel: string | null
  session: BrowserSession | null
  signal?: AbortSignal
  defaultTabGroupId: string | null
  flags: { newPage: boolean; closePage: boolean; listTabs: boolean }
}

export type ToolGuard = (call: ToolCall) => ToolResult | null

export interface ToolEffectContext {
  call: ToolCall
  result: ToolResult
  cancelled: boolean
  durationMs: number
}

export type ToolEffect = (context: ToolEffectContext) => ToolResult | undefined

export interface NamedToolEffect {
  name: string
  run: ToolEffect
}

const GUARDS: readonly ToolGuard[] = [
  guardNavigateScheme,
  guardBrowserConnected,
  guardPageOwnership,
]

const BASE_EFFECTS: readonly NamedToolEffect[] = [
  { name: 'ownership-claims', run: applyOwnershipClaims },
  { name: 'tabs-list-view', run: applyTabsListView },
  { name: 'audit', run: applyAudit },
  { name: 'tab-activity', run: applyTabActivity },
  { name: 'tab-groups', run: applyTabGroups },
  // Must stay last: tabs-list-view rewrites result content wholesale, so a
  // nudge appended before it would be clobbered.
  { name: 'session-naming', run: applySessionNaming },
]

/** Returns the first guard rejection in pipeline order. */
export function runGuards(
  call: ToolCall,
  guards: readonly ToolGuard[] = GUARDS,
): ToolResult | null {
  for (const guard of guards) {
    const rejection = guard(call)
    if (rejection) return rejection
  }
  return null
}

/** Runs effects in order, retaining the latest result when an effect fails. */
export function runEffects(
  context: ToolEffectContext,
  effects: readonly NamedToolEffect[] = BASE_EFFECTS,
  warn = logger.warn,
): ToolResult {
  let result = context.result
  for (const effect of effects) {
    try {
      result = effect.run({ ...context, result }) ?? result
    } catch (error) {
      warn('cockpit tool dispatch effect failed', {
        tool: context.call.tool.name,
        sessionId: context.call.sessionId || undefined,
        effect: effect.name,
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }
  return result
}

/** Registers the browser tool catalogue on the shared MCP server. */
export function registerBrowserToolsForSingleServer(
  server: McpServer,
  resolveIdentity: (sessionId: string | undefined) => ClientIdentity | null,
): void {
  const register = asRegister(server)
  for (const tool of BROWSER_TOOLS) {
    register(
      tool.name,
      {
        description: tool.description,
        // The tool's zod shape is v3 while the SDK wrapper uses v4; runtime JSON Schema is compatible.
        inputSchema: tool.input.shape as unknown as ZodRawShape,
        ...(tool.annotations && {
          annotations: tool.annotations as Record<string, unknown>,
        }),
      },
      (args, extra) =>
        dispatchToolCall(buildToolCall(tool, args, extra, resolveIdentity)),
    )
  }
  register(
    'name_session',
    {
      description:
        'Rename this browser session: a small lowercase 2-3 word label for what this session is doing, e.g. "invoice processing". Tabs are grouped as <client>/<name>. Call again to rename.',
      inputSchema: { name: z.string().max(64) },
      annotations: {
        title: 'Name session',
        readOnlyHint: false,
        destructiveHint: false,
        idempotentHint: true,
      },
    },
    async (args, extra) => {
      const identity = resolveIdentity(extra?.sessionId)
      if (!identity) return errorResult('unable to resolve this session')
      const label = normalizeSmallName(args.name as string)
      if (!label) return errorResult('name must contain a usable session name')

      const prefix = clientPrefixFromSlug(identity.slug)
      const oldTitle = buildSessionGroupTitle(prefix, identity.label)
      const newTitle = buildSessionGroupTitle(prefix, label)
      identityService.setLabel(identity.sessionId, label)
      await applyAgentTabGroupTitle({
        key: identity.key,
        title: newTitle,
        session: getBrowserSession(),
      })
      return textResult(`renamed to ${newTitle} (was ${oldTitle})`)
    },
  )
}

interface DispatchExtra {
  signal?: AbortSignal
  sessionId?: string
}

interface ExecutionOutcome {
  result: ToolResult
  cancelled: boolean
  durationMs: number
}

type ConnectedToolCall = ToolCall & { session: BrowserSession }

function buildToolCall(
  tool: ToolDefinition,
  args: unknown,
  extra: DispatchExtra | undefined,
  resolveIdentity: (sessionId: string | undefined) => ClientIdentity | null,
): ToolCall {
  const identity = resolveIdentity(extra?.sessionId)
  const agent = identity ? agentIdentityFromClient(identity) : null
  const key = identity ? agentKeyFromClient(identity) : null
  const defaultTabGroupId = key
    ? (ownershipStore.groupOf(key)?.id ?? null)
    : null
  const action =
    tool.name === 'tabs'
      ? ((args as { action?: unknown } | null | undefined)?.action ?? 'list')
      : null
  return {
    tool,
    args,
    sessionId: extra?.sessionId ?? '',
    identity,
    key,
    agent,
    agentLabel: identity
      ? identity.clientTitle && identity.clientTitle.length > 0
        ? identity.clientTitle
        : identity.clientName.length > 0
          ? identity.clientName
          : (agent?.slug ?? null)
      : null,
    session: getBrowserSession(),
    signal: extra?.signal,
    defaultTabGroupId,
    flags: {
      newPage: action === 'new',
      closePage: action === 'close',
      listTabs: action === 'list',
    },
  }
}

async function dispatchToolCall(call: ToolCall): Promise<ToolResult> {
  const rejection = runGuards(call)
  if (rejection) return wireResult(rejection)
  const connectedCall = call as ConnectedToolCall

  if (ARBITRARY_SCRIPT_TOOLS.has(call.tool.name)) {
    logger.warn('cockpit dispatched arbitrary-script tool', {
      tool: call.tool.name,
      sessionId: call.sessionId || undefined,
    })
  }

  const outcome = await executeWithCancellation(connectedCall)
  if (outcome.result.isError && !outcome.cancelled) {
    logger.warn('cockpit tool dispatch failed', {
      tool: call.tool.name,
      sessionId: call.sessionId || undefined,
      durationMs: outcome.durationMs,
      error: dispatchErrorText(outcome.result.content),
    })
  }
  const result = runEffects({ call, ...outcome })
  return wireResult(result)
}

function wireResult(result: ToolResult): ToolResult {
  return {
    content: result.content,
    isError: result.isError,
  }
}

/** Executes a connected call while composing client and operator cancellation. */
async function executeWithCancellation(
  call: ConnectedToolCall,
): Promise<ExecutionOutcome> {
  const dispatchStart = Date.now()
  const userCancel = new AbortController()
  if (call.sessionId) dispatchCancellation.register(call.sessionId, userCancel)
  const signal = composeAbortSignals([call.signal, userCancel.signal])

  let result: ToolResult
  try {
    result = await executeTool(call.tool, call.args, {
      session: call.session,
      signal,
      defaultTabGroupId: call.defaultTabGroupId ?? undefined,
    })
  } catch (error) {
    if (userCancel.signal.aborted) {
      result = cancellationErrorResult(CANCELLATION_REASON)
    } else {
      logThrownDispatch(call, dispatchStart, error)
      throw error
    }
  } finally {
    if (call.sessionId) {
      dispatchCancellation.unregister(call.sessionId, userCancel)
    }
  }

  if (userCancel.signal.aborted) {
    result = cancellationErrorResult(CANCELLATION_REASON)
  }
  return {
    result,
    cancelled: userCancel.signal.aborted,
    durationMs: Date.now() - dispatchStart,
  }
}

function logThrownDispatch(
  call: ToolCall,
  dispatchStart: number,
  error: unknown,
): void {
  const fields = {
    tool: call.tool.name,
    sessionId: call.sessionId || undefined,
    durationMs: Date.now() - dispatchStart,
  }
  if (call.signal?.aborted) {
    logger.info('cockpit tool dispatch cancelled by client', fields)
    return
  }
  logger.error('cockpit tool dispatch threw', {
    ...fields,
    error: error instanceof Error ? error.message : String(error),
  })
}
