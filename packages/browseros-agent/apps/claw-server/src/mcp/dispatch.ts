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
  executeTool,
  type ToolDefinition,
} from '@browseros/browser-mcp/tools/framework'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { ZodRawShape } from 'zod'
import type { AgentKey } from '../domain/agent-key'
import { ownershipStore } from '../domain/ownership'
import { getBrowserSession } from '../lib/browser-session'
import { logger } from '../lib/logger'
import {
  agentIdentityFromClient,
  agentKeyFromClient,
  type ClientIdentity,
} from '../lib/mcp-session'
import {
  CANCELLATION_REASON,
  dispatchCancellation,
} from '../services/dispatch-cancellation'
import { cancellationErrorResult } from './cancellation-result'
import { composeAbortSignals, dispatchErrorText } from './dispatch-util'
import { applyAudit } from './effects/audit'
import { applyOwnershipClaims } from './effects/ownership-claims'
import { createSessionNamingEffect } from './effects/session-naming'
import { applyTabActivity } from './effects/tab-activity'
import { applyTabGroups } from './effects/tab-groups'
import { applyTabsListView } from './effects/tabs-list-view'
import { guardBrowserConnected } from './guards/browser-connected'
import { guardNavigateScheme } from './guards/navigate-scheme'
import { guardPageOwnership } from './guards/page-ownership'
import { asRegister, type ToolResult } from './register-fn'
import type { SessionNamingServer } from './session-naming'

const ARBITRARY_SCRIPT_TOOLS = new Set(['run', 'evaluate'])

export interface ToolCall {
  tool: ToolDefinition
  args: unknown
  sessionId: string
  requestId: unknown
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
      warn('cockpit v2 tool dispatch effect failed', {
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
  options: { sessionNamingServer: SessionNamingServer },
): void {
  const register = asRegister(server)
  const effects: readonly NamedToolEffect[] = [
    ...BASE_EFFECTS,
    {
      name: 'session-naming',
      run: createSessionNamingEffect(options.sessionNamingServer),
    },
  ]
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
        dispatchToolCall(
          buildToolCall(tool, args, extra, resolveIdentity),
          effects,
        ),
    )
  }
}

interface DispatchExtra {
  signal?: AbortSignal
  sessionId?: string
  requestId?: string | number
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
    requestId: extra?.requestId,
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

async function dispatchToolCall(
  call: ToolCall,
  effects: readonly NamedToolEffect[],
): Promise<ToolResult> {
  const rejection = runGuards(call)
  if (rejection) return wireResult(rejection)
  const connectedCall = call as ConnectedToolCall

  if (ARBITRARY_SCRIPT_TOOLS.has(call.tool.name)) {
    logger.warn('cockpit v2 dispatched arbitrary-script tool', {
      tool: call.tool.name,
      sessionId: call.sessionId || undefined,
    })
  }

  const outcome = await executeWithCancellation(connectedCall)
  if (outcome.result.isError && !outcome.cancelled) {
    logger.warn('cockpit v2 tool dispatch failed', {
      tool: call.tool.name,
      sessionId: call.sessionId || undefined,
      durationMs: outcome.durationMs,
      error: dispatchErrorText(outcome.result.content),
    })
  }
  const result = runEffects({ call, ...outcome }, effects)
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
    logger.info('cockpit v2 tool dispatch cancelled by client', fields)
    return
  }
  logger.error('cockpit v2 tool dispatch threw', {
    ...fields,
    error: error instanceof Error ? error.message : String(error),
  })
}
