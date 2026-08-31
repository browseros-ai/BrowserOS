/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Authoritative browser-tool dispatch:
 *
 *   MCP adapter -> guards -> execute -> effects -> observers
 *
 * Guards may reject, effects may enrich server state, and observers only
 * report. Effect and observer failures never replace the browser-tool result.
 */

import type { BrowserSession } from '@browseros/browser-core/core/session'
import type { BrowserOutputFileAccess } from '@browseros/browser-mcp/output-file'
import { withBrowserOutputFileAccess } from '@browseros/browser-mcp/output-file'
import {
  executeTool,
  type ToolContext,
  type ToolDefinition,
  type ToolResult,
} from '@browseros/browser-mcp/tools/framework'
import { logger } from '../../../lib/logger'
import type { ActiveConversationRun } from '../conversation-runs'
import type { ConversationTabGroups } from './conversation-tab-groups'
import { applyConversationTabs } from './effects/conversation-tabs'
import { applyTabGroups } from './effects/tab-groups'
import { guardConversationRunning } from './guards/conversation-running'
import { guardReadOnly } from './guards/read-only'
import { observeToolTelemetry } from './observers/tool-telemetry'

const BROWSER_TOOL_TIMEOUT_MS = 120_000

export interface BrowserMcpLeaseScope {
  conversationId: string
  outputFileAccess: BrowserOutputFileAccess
}

export interface BrowserToolPageTrace {
  /** Every page the tool interacted with, including pages it created. */
  readonly touched: Set<number>
  /** Only pages created by this call; these are eligible for agent grouping. */
  readonly created: Set<number>
}

export interface BrowserToolCall {
  tool: ToolDefinition
  args: Record<string, unknown>
  context: ToolContext
  lease?: BrowserMcpLeaseScope
  run?: ActiveConversationRun
  readOnly: boolean
  source: string
  trace: BrowserToolPageTrace
  tabGroups: Pick<ConversationTabGroups, 'addCreatedPages'>
}

export interface BrowserToolEffectContext {
  call: BrowserToolCall
  result: ToolResult
  durationMs: number
}

export interface BrowserToolObserverContext {
  call: BrowserToolCall
  result?: ToolResult
  error?: unknown
  durationMs: number
}

export type BrowserToolGuard = (call: BrowserToolCall) => ToolResult | null
export type BrowserToolEffect = (
  context: BrowserToolEffectContext,
) => ToolResult | undefined | Promise<ToolResult | undefined>
export type BrowserToolObserver = (
  context: BrowserToolObserverContext,
) => void | Promise<void>

interface NamedToolGuard {
  name: string
  run: BrowserToolGuard
}

interface NamedToolEffect {
  name: string
  run: BrowserToolEffect
}

interface NamedToolObserver {
  name: string
  run: BrowserToolObserver
}

const GUARDS: readonly NamedToolGuard[] = [
  { name: 'conversation-running', run: guardConversationRunning },
  { name: 'read-only', run: guardReadOnly },
]

const EFFECTS: readonly NamedToolEffect[] = [
  { name: 'conversation-tabs', run: applyConversationTabs },
  { name: 'tab-groups', run: applyTabGroups },
]

const OBSERVERS: readonly NamedToolObserver[] = [
  { name: 'telemetry', run: observeToolTelemetry },
]

/** Dispatches one browser tool through the ordered server-owned pipeline. */
export async function dispatchBrowserTool(
  input: Omit<BrowserToolCall, 'trace'>,
): Promise<ToolResult> {
  const call: BrowserToolCall = {
    ...input,
    trace: { touched: new Set(), created: new Set() },
  }
  const startedAt = performance.now()
  const durationMs = () => Math.round(performance.now() - startedAt)

  const rejection = runGuards(call)
  if (rejection) {
    await runObservers({ call, result: rejection, durationMs: durationMs() })
    return rejection
  }

  try {
    const result = await executeBrowserTool(call)
    collectDeclaredPageEffects(call, result)
    const effected = await runEffects({
      call,
      result,
      durationMs: durationMs(),
    })
    await runObservers({ call, result: effected, durationMs: durationMs() })
    return effected
  } catch (error) {
    await runObservers({ call, error, durationMs: durationMs() })
    throw error
  }
}

/** Returns the first rejection in declared guard order. */
export function runGuards(
  call: BrowserToolCall,
  guards: readonly NamedToolGuard[] = GUARDS,
): ToolResult | null {
  for (const guard of guards) {
    const rejection = guard.run(call)
    if (rejection) return rejection
  }
  return null
}

/** Runs ordered effects while preserving the latest valid tool result. */
export async function runEffects(
  context: BrowserToolEffectContext,
  effects: readonly NamedToolEffect[] = EFFECTS,
): Promise<ToolResult> {
  let result = context.result
  for (const effect of effects) {
    try {
      result = (await effect.run({ ...context, result })) ?? result
    } catch (error) {
      logger.warn('Browser MCP effect failed', {
        tool: context.call.tool.name,
        conversationId: context.call.run?.conversationId,
        effect: effect.name,
        error: errorText(error),
      })
    }
  }
  return result
}

async function runObservers(
  context: BrowserToolObserverContext,
  observers: readonly NamedToolObserver[] = OBSERVERS,
): Promise<void> {
  for (const observer of observers) {
    try {
      await observer.run(context)
    } catch (error) {
      logger.warn('Browser MCP observer failed', {
        tool: context.call.tool.name,
        conversationId: context.call.run?.conversationId,
        observer: observer.name,
        error: errorText(error),
      })
    }
  }
}

async function executeBrowserTool(call: BrowserToolCall): Promise<ToolResult> {
  const signals = [
    call.context.signal,
    call.run?.signal,
    AbortSignal.timeout(BROWSER_TOOL_TIMEOUT_MS),
  ].filter((signal): signal is AbortSignal => signal !== undefined)
  const signal = signals.length === 1 ? signals[0] : AbortSignal.any(signals)
  const session = call.run
    ? trackBrowserSessionPages(call.context.session, call.trace)
    : call.context.session

  return await withBrowserOutputFileAccess(call.lease?.outputFileAccess, () =>
    executeTool(call.tool, call.args, {
      ...call.context,
      session,
      signal,
    }),
  )
}

function collectDeclaredPageEffects(
  call: BrowserToolCall,
  result: ToolResult,
): void {
  const action =
    typeof call.args.action === 'string' ? call.args.action : undefined
  // A list observes browser-wide metadata; it does not touch every returned tab.
  if (call.tool.name === 'tabs' && (action ?? 'list') === 'list') return

  if (typeof call.args.page === 'number') {
    call.trace.touched.add(call.args.page)
  }
  if (Array.isArray(call.args.pages)) {
    for (const page of call.args.pages) {
      if (typeof page === 'number') call.trace.touched.add(page)
    }
  }

  if (call.tool.name !== 'tabs') return
  const page = asRecord(result.structuredContent)?.page
  const pageId =
    typeof page === 'number'
      ? page
      : typeof asRecord(page)?.pageId === 'number'
        ? (asRecord(page)?.pageId as number)
        : undefined
  if (pageId === undefined) return
  call.trace.touched.add(pageId)
  if (action === 'new') call.trace.created.add(pageId)
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
 * Wraps one call's BrowserSession so the open-ended `run` tool emits page facts
 * without mutating the shared session or leaking data across conversations.
 */
function trackBrowserSessionPages(
  session: BrowserSession,
  trace: BrowserToolPageTrace,
): BrowserSession {
  const touch = (pageId: number) => trace.touched.add(pageId)
  const trackedPages = new Proxy(session.pages, {
    get(target, property) {
      const value = Reflect.get(target, property, target)
      if (typeof value !== 'function') return value

      if (property === 'newPage') {
        return async (...args: unknown[]) => {
          const pageId = await Reflect.apply(value, target, args)
          if (typeof pageId === 'number') {
            touch(pageId)
            trace.created.add(pageId)
          }
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

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
