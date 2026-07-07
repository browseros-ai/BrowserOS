/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Wires every browser tool from `@browseros/browser-mcp`'s catalogue onto
 * a per-agent MCP server. Each dispatch:
 *
 *   1. Applies the hard navigate URL-scheme guard.
 *   2. Looks up the live BrowserSession; if not yet wired, returns
 *      a structured "session not connected" error so the wire shape
 *      stays honest.
 *   3. Hands off to `executeTool` from `@browseros/browser-mcp`'s tool
 *      framework. That handles arg validation, error formatting,
 *      tab-id metadata, and result composition.
 */

import { BROWSER_TOOLS } from '@browseros/browser-mcp/registry'
import { executeTool } from '@browseros/browser-mcp/tools/framework'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { ZodRawShape } from 'zod'
import { agentTabs } from '../lib/agent-tabs'
import { getBrowserSession } from '../lib/browser-session'
import { logger } from '../lib/logger'
import {
  agentIdentityFromClient,
  type ClientIdentity,
  identityService,
} from '../lib/mcp-session'
import {
  extractPageId,
  TOOLS_WITH_PAGE,
  tabActivityRegistry,
} from '../lib/tab-activity'
import type { StoredAgentProfile } from '../routes/agents/schemas'
import { recordToolDispatch } from '../services/audit-log'
import {
  CANCELLATION_REASON,
  dispatchCancellation,
} from '../services/dispatch-cancellation'
import { persistScreenshot } from '../services/screenshots'
import { ensureAgentTabGroup } from '../services/tab-group-ops'
import { cancellationErrorResult } from './cancellation-result'
import { asRegister, type ToolResult } from './register-fn'
import {
  annotateTabsListWithOwnership,
  buildOwnershipDeps,
} from './tabs-ownership'

/**
 * Schemes the cockpit refuses to forward to `navigate`, regardless of
 * what the parent server's tool schema would accept. The real navigate
 * tool's zod input is `z.string().optional()` with no scheme check, so
 * without this guard a `javascript:`, `file:`, or `data:` URL would reach
 * the CDP layer.
 */
const NAVIGATE_BLOCKED_SCHEMES = new Set(['javascript:', 'file:', 'data:'])

const ARBITRARY_SCRIPT_TOOLS = new Set(['run', 'evaluate'])

/**
 * Records a successful dispatch into the tab-activity registry. The
 * homepage attributes the tab to the agent and surfaces the latest
 * tool name. Failed dispatches and tools without a `page` arg are
 * skipped at the call site by `extractPageId` returning `null`.
 */
function recordSuccessfulDispatch(args: {
  toolName: string
  rawArgs: unknown
  agent: StoredAgentProfile
  session: ReturnType<typeof getBrowserSession>
}): void {
  if (!args.session) return
  const pageId = extractPageId(args.toolName, args.rawArgs)
  if (pageId === null) return
  const live = args.session.pages.getInfo(pageId)
  if (!live) return
  tabActivityRegistry.recordTool({
    agentId: args.agent.id,
    slug: args.agent.slug,
    pageId,
    targetId: live.targetId,
    toolName: args.toolName,
  })
}

export function registerBrowserTools(
  server: McpServer,
  agent: StoredAgentProfile,
): void {
  const register = asRegister(server)
  for (const tool of BROWSER_TOOLS) {
    register(
      tool.name,
      {
        description: tool.description,
        // The tool's zod shape is v3 (apps/server's pin); our SDK
        // wrapper is typed against v4. Runtime is compatible — both
        // produce equivalent JSON Schema for the shapes in use here.
        // Cast at the boundary keeps the mismatch isolated.
        inputSchema: tool.input.shape as unknown as ZodRawShape,
        ...(tool.annotations && {
          annotations: tool.annotations as Record<string, unknown>,
        }),
      },
      async (rawArgs, extra) => {
        if (tool.name === 'navigate') {
          const url = (rawArgs as { url?: unknown } | null | undefined)?.url
          if (typeof url === 'string' && url.length > 0) {
            const scheme = url.slice(0, url.indexOf(':') + 1).toLowerCase()
            if (NAVIGATE_BLOCKED_SCHEMES.has(scheme)) {
              return {
                content: [
                  {
                    type: 'text',
                    text: `navigate refuses ${scheme} URLs; only http(s) is allowed`,
                  },
                ],
                isError: true,
              } satisfies ToolResult
            }
          }
        }

        const session = getBrowserSession()
        if (!session) {
          return {
            content: [
              {
                type: 'text',
                text: 'browser session not connected; the cockpit runtime has not been wired to a live Chromium yet',
              },
            ],
            isError: true,
          } satisfies ToolResult
        }

        if (ARBITRARY_SCRIPT_TOOLS.has(tool.name)) {
          logger.warn('cockpit dispatched arbitrary-script tool', {
            tool: tool.name,
            agentId: agent.id,
          })
        }
        const result = await executeTool(tool, rawArgs, {
          session,
          signal: extra?.signal,
        })
        if (!result.isError) {
          recordSuccessfulDispatch({
            toolName: tool.name,
            rawArgs,
            agent,
            session,
          })
        }
        return {
          content: result.content,
          isError: result.isError,
          structuredContent: result.structuredContent,
        }
      },
    )
  }
}

/**
 * Combine zero or more AbortSignals into one. Returns:
 *  - `undefined` when no inputs are supplied (no abort wiring)
 *  - the single input when only one is supplied (avoids the
 *    AbortSignal.any wrapper overhead in the common case)
 *  - an AbortSignal.any of all defined inputs otherwise
 *
 * AbortSignal.any is supported in Node 20.3+ and Bun runtimes the
 * cockpit targets. Each input is dropped if it is undefined so
 * callers can pass `[extra?.signal, userCancel.signal]` without
 * filtering first.
 */
function composeAbortSignals(
  signals: ReadonlyArray<AbortSignal | undefined>,
): AbortSignal | undefined {
  const defined = signals.filter((s): s is AbortSignal => s !== undefined)
  if (defined.length === 0) return undefined
  if (defined.length === 1) return defined[0]
  return AbortSignal.any(defined)
}

/**
 * First text block of a failed dispatch result, capped so a long
 * tool message cannot bloat the log line. executeTool's error
 * results carry their human-readable reason here.
 */
const DISPATCH_ERROR_TEXT_MAX = 200

function dispatchErrorText(content: unknown): string | null {
  if (!Array.isArray(content)) return null
  for (const block of content) {
    if (
      block !== null &&
      typeof block === 'object' &&
      (block as { type?: unknown }).type === 'text' &&
      typeof (block as { text?: unknown }).text === 'string'
    ) {
      return (block as { text: string }).text.slice(0, DISPATCH_ERROR_TEXT_MAX)
    }
  }
  return null
}

/**
 * v2 dispatch record helper. The single MCP endpoint does not know
 * which `StoredAgentProfile` produced the call, so the registry write
 * sources its identity from the per-session `ClientIdentity` instead.
 * The shape matches the legacy `recordSuccessfulDispatch` so the
 * homepage / rollup / trail wiring stays unchanged.
 */
function recordSuccessfulDispatchV2(args: {
  toolName: string
  rawArgs: unknown
  identity: ClientIdentity
  session: ReturnType<typeof getBrowserSession>
}): void {
  if (!args.session) return
  const pageId = extractPageId(args.toolName, args.rawArgs)
  if (pageId === null) return
  const live = args.session.pages.getInfo(pageId)
  if (!live) return
  const { agentId, slug } = agentIdentityFromClient(args.identity)
  tabActivityRegistry.recordTool({
    agentId,
    slug,
    pageId,
    targetId: live.targetId,
    toolName: args.toolName,
  })
}

/**
 * Registers the same browser-tool catalogue against the v2 single
 * MCP server. The per-tool dispatch reads the connecting client's
 * identity from `extra.sessionId` via the supplied resolver so the
 * tab-activity registry can attribute calls to specific agents even
 * though every agent shares the same endpoint.
 *
 * The navigate-scheme guard stays because it is a hard security check on
 * the URL shape, not a per-agent policy.
 */
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
        inputSchema: tool.input.shape as unknown as ZodRawShape,
        ...(tool.annotations && {
          annotations: tool.annotations as Record<string, unknown>,
        }),
      },
      async (rawArgs, extra) => {
        if (tool.name === 'navigate') {
          const url = (rawArgs as { url?: unknown } | null | undefined)?.url
          if (typeof url === 'string' && url.length > 0) {
            const scheme = url.slice(0, url.indexOf(':') + 1).toLowerCase()
            if (NAVIGATE_BLOCKED_SCHEMES.has(scheme)) {
              // Rejections before dispatchStart get their own message
              // (vs 'dispatch failed'): nothing was executed, and an
              // agent probing javascript:/file:/data: URLs is signal
              // worth spotting on its own.
              logger.warn('cockpit v2 tool dispatch rejected', {
                tool: tool.name,
                sessionId: extra?.sessionId,
                reason: 'blocked navigate scheme',
              })
              return {
                content: [
                  {
                    type: 'text',
                    text: `navigate refuses ${scheme} URLs; only http(s) is allowed`,
                  },
                ],
                isError: true,
              } satisfies ToolResult
            }
          }
        }

        const session = getBrowserSession()
        if (!session) {
          // Every call an agent makes while the cockpit runs without
          // an attached BrowserOS lands here; without this line the
          // only trace is the single boot-time bootstrap warn.
          logger.warn('cockpit v2 tool dispatch rejected', {
            tool: tool.name,
            sessionId: extra?.sessionId,
            reason: 'browser session not connected',
          })
          return {
            content: [
              {
                type: 'text',
                text: 'browser session not connected; the cockpit runtime has not been wired to a live Chromium yet',
              },
            ],
            isError: true,
          } satisfies ToolResult
        }

        if (ARBITRARY_SCRIPT_TOOLS.has(tool.name)) {
          // Same audit log as the per-agent path; identity is the
          // mcp-session id when the client did not name itself.
          logger.warn('cockpit v2 dispatched arbitrary-script tool', {
            tool: tool.name,
            sessionId: extra?.sessionId,
          })
        }

        // Cross-agent page guard. Reject dispatches whose `page` arg
        // points at a tab this agent does not own so an agent can
        // only touch tabs it opened via `tabs new`. Prevents the
        // "codex takes over the operator's active tab" failure
        // mode: without this guard, an agent that sees a page id
        // from anywhere (its LLM cache, `tabs active`, a prior
        // session) could dispatch snapshot / navigate / etc. on the
        // operator's tab. Fires BEFORE executeTool so the underlying
        // tool never sees the bad page id. Fail-open when identity
        // is unknown (unusual; matches the rest of the dispatch
        // path's identity-optional behaviour).
        if (TOOLS_WITH_PAGE.has(tool.name)) {
          const pageArg = (rawArgs as { page?: unknown } | null | undefined)
            ?.page
          if (
            typeof pageArg === 'number' &&
            Number.isInteger(pageArg) &&
            pageArg >= 1
          ) {
            const guardIdentity = resolveIdentity(extra?.sessionId)
            if (guardIdentity) {
              const { agentId: guardAgentId } =
                agentIdentityFromClient(guardIdentity)
              if (!agentTabs.ownedBy(guardAgentId).has(pageArg)) {
                logger.warn('cockpit v2 rejected foreign-page dispatch', {
                  tool: tool.name,
                  sessionId: extra?.sessionId,
                  agentId: guardAgentId,
                  page: pageArg,
                })
                return {
                  content: [
                    {
                      type: 'text',
                      text: `page ${pageArg} is not owned by this agent; call \`tabs new\` to open a fresh page and use the returned page id.`,
                    },
                  ],
                  isError: true,
                } satisfies ToolResult
              }
            }
          }
        }

        const dispatchStart = Date.now()

        // Operator-cancel hook. Compose the transport's existing
        // signal (client-driven notifications/cancelled) with our
        // own so EITHER side firing aborts executeTool cleanly. The
        // controller is registered before the call and unregistered
        // in the finally block so a successful or errored dispatch
        // never leaves a stale entry behind.
        const userCancel = new AbortController()
        const sessionId = extra?.sessionId ?? ''
        if (sessionId) dispatchCancellation.register(sessionId, userCancel)
        const composedSignal = composeAbortSignals([
          extra?.signal,
          userCancel.signal,
        ])

        let result: Awaited<ReturnType<typeof executeTool>>
        try {
          result = await executeTool(tool, rawArgs, {
            session,
            signal: composedSignal,
          })
        } catch (err) {
          if (userCancel.signal.aborted) {
            result = cancellationErrorResult(CANCELLATION_REASON)
          } else {
            // A client-driven abort (notifications/cancelled, harness
            // timeout) is normal lifecycle, not a server fault; only a
            // throw with no abort anywhere is a genuine failure.
            if (extra?.signal?.aborted) {
              logger.info('cockpit v2 tool dispatch cancelled by client', {
                tool: tool.name,
                sessionId: extra?.sessionId,
                durationMs: Date.now() - dispatchStart,
              })
            } else {
              logger.error('cockpit v2 tool dispatch threw', {
                tool: tool.name,
                sessionId: extra?.sessionId,
                durationMs: Date.now() - dispatchStart,
                error: err instanceof Error ? err.message : String(err),
              })
            }
            throw err
          }
        } finally {
          if (sessionId) dispatchCancellation.unregister(sessionId, userCancel)
        }
        // Some tools translate an abort into a structured isError
        // result rather than throwing; cover that too so the operator
        // attribution is honest in the audit log.
        if (userCancel.signal.aborted) {
          result = cancellationErrorResult(CANCELLATION_REASON)
        }
        const durationMs = Date.now() - dispatchStart

        // Failed dispatches are otherwise invisible server-side: the
        // audit DB rows land only for successes and operator cancels,
        // and the isError result rides back to the agent's harness.
        if (result.isError && !userCancel.signal.aborted) {
          logger.warn('cockpit v2 tool dispatch failed', {
            tool: tool.name,
            sessionId: extra?.sessionId,
            durationMs,
            error: dispatchErrorText(result.content),
          })
        }

        // Record cancelled dispatches in the audit log so the task
        // timeline shows the operator's intervention. The existing
        // success branch below is left untouched; cancellations are
        // tracked here with a small adapter that walks the same
        // recordToolDispatch path with isError: true.
        if (userCancel.signal.aborted) {
          const identity = resolveIdentity(extra?.sessionId)
          if (identity) {
            const { agentId, slug } = agentIdentityFromClient(identity)
            const agentLabel =
              identity.clientTitle && identity.clientTitle.length > 0
                ? identity.clientTitle
                : identity.clientName.length > 0
                  ? identity.clientName
                  : slug
            const pageId = extractPageId(tool.name, rawArgs)
            const live = pageId !== null ? session.pages.getInfo(pageId) : null
            recordToolDispatch({
              agentId,
              slug,
              agentLabel,
              sessionId: extra?.sessionId ?? '',
              toolName: tool.name,
              pageId,
              targetId: live?.targetId ?? null,
              url: live?.url ?? null,
              title: live?.title ?? null,
              rawArgs,
              durationMs,
              result: {
                isError: true,
                structuredContent: result.structuredContent,
                content: result.content,
              },
            })
          }
        }

        if (!result.isError) {
          const identity = resolveIdentity(extra?.sessionId)
          if (identity) {
            recordSuccessfulDispatchV2({
              toolName: tool.name,
              rawArgs,
              identity,
              session,
            })
            // v2 audit log: persist every successful dispatch to
            // SQLite. Snapshot agentLabel, url, title at dispatch
            // time so renames / navigations later do not rewrite
            // history. Best-effort write; never blocks the agent.
            const { agentId, slug } = agentIdentityFromClient(identity)
            const agentLabel =
              identity.clientTitle && identity.clientTitle.length > 0
                ? identity.clientTitle
                : identity.clientName.length > 0
                  ? identity.clientName
                  : slug
            const pageId = extractPageId(tool.name, rawArgs)
            const live = pageId !== null ? session.pages.getInfo(pageId) : null
            const dispatchId = recordToolDispatch({
              agentId,
              slug,
              agentLabel,
              sessionId: extra?.sessionId ?? '',
              toolName: tool.name,
              pageId: pageId,
              targetId: live?.targetId ?? null,
              url: live?.url ?? null,
              title: live?.title ?? null,
              rawArgs,
              durationMs,
              result: {
                isError: result.isError ?? false,
                structuredContent: result.structuredContent,
                content: result.content,
              },
            })
            if (dispatchId !== null) {
              // `tabs new` is the one page-targeted tool whose page
              // id is only born in the RESULT (not in args). Prefer
              // the result-derived value so the screencast fallback
              // + first-capture policy see the right pageId.
              let screenshotPageId: number | null = pageId
              if (tool.name === 'tabs') {
                const args = rawArgs as { action?: string } | null | undefined
                if (args?.action === 'new') {
                  const resultPageId = (
                    result.structuredContent as { page?: number } | undefined
                  )?.page
                  if (typeof resultPageId === 'number') {
                    screenshotPageId = resultPageId
                  }
                }
              }
              persistScreenshot({
                dispatchId,
                toolName: tool.name,
                pageId: screenshotPageId,
                agentId,
                result: {
                  isError: result.isError ?? false,
                  content: result.content,
                  structuredContent: result.structuredContent,
                },
              })
            }
            // v2 cockpit-owned tab grouping: when the agent opens a
            // new tab, auto-add it to the agent's tab group. The
            // orchestrator handles create-on-first-call and
            // serialises across racing tabs/new dispatches.
            if (tool.name === 'tabs') {
              const args = rawArgs as { action?: string } | null | undefined
              if (args?.action === 'new') {
                const pageId = (
                  result.structuredContent as { page?: number } | undefined
                )?.page
                if (typeof pageId === 'number') {
                  const { agentId, slug } = agentIdentityFromClient(identity)
                  // tabs new carries no `page` field in its input
                  // args; the page id is born in the dispatch result.
                  // recordSuccessfulDispatchV2 above therefore
                  // skipped the registry write (extractPageId
                  // returned null). Record here using the result-
                  // derived pageId so /tabs/activity reflects the
                  // new tab the moment it opens, not when a later
                  // page-targeted dispatch (snapshot / navigate)
                  // happens to land on it.
                  const live = session.pages.getInfo(pageId)
                  if (live) {
                    tabActivityRegistry.recordTool({
                      agentId,
                      slug,
                      pageId,
                      targetId: live.targetId,
                      toolName: 'tabs',
                    })
                  }
                  // Isolation ledger: this page now belongs to this
                  // agent. Subsequent page-targeted dispatches will
                  // pass the cross-agent page guard for this id and
                  // fail it for any other agent's session.
                  agentTabs.markOpened(agentId, pageId)
                  void ensureAgentTabGroup({
                    agentId,
                    slug,
                    pageId,
                    session,
                    signal: extra?.signal,
                  })
                }
              } else if (args?.action === 'close') {
                // Drop the closed page from the isolation ledger so
                // the agent cannot re-reference it and so `tabs list`
                // stops surfacing it.
                const closedPage = (rawArgs as { page?: unknown } | null)?.page
                if (
                  typeof closedPage === 'number' &&
                  Number.isInteger(closedPage) &&
                  closedPage >= 1
                ) {
                  const { agentId } = agentIdentityFromClient(identity)
                  agentTabs.markClosed(agentId, closedPage)
                }
              } else if ((args?.action ?? 'list') === 'list') {
                // Annotate every open page with an ownership tag and
                // group the text output into Your tabs / User's tabs
                // / Other agents' tabs. Every page in the underlying
                // result survives; the cross-agent page guard above
                // still hard-rejects a page-targeted dispatch on a
                // foreign page, so broader visibility does not open
                // a broader action surface.
                const deps = buildOwnershipDeps(
                  identity,
                  agentTabs,
                  identityService,
                )
                result = annotateTabsListWithOwnership(result, deps)
              }
            }
          } else {
            // Initialize was skipped or the session id is unknown;
            // the dispatch still succeeded but the homepage will not
            // see this call. Log so the operator can diagnose.
            logger.warn('cockpit v2 dispatch missing identity', {
              tool: tool.name,
              sessionId: extra?.sessionId,
            })
          }
        }

        return {
          content: result.content,
          isError: result.isError,
          structuredContent: result.structuredContent,
        }
      },
    )
  }
}
