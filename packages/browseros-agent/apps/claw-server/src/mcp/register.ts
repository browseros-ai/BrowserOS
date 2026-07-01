/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Wires every browser tool from `@browseros/browser-mcp`'s catalogue onto
 * a per-agent MCP server with a permission gate in front. Each
 * dispatch:
 *
 *   1. Maps the tool name to a permission verb in the cockpit's
 *      catalog space.
 *   2. Looks up a domain hint from the agent (real per-page URL
 *      tracking is a future-phase concern; today we use the agent's
 *      first declared site).
 *   3. Calls `permissions.check(agent, verb, domain)` and
 *      short-circuits on `block` / `ask`.
 *   4. Looks up the live BrowserSession; if not yet wired, returns
 *      a structured "session not connected" error so the wire shape
 *      stays honest.
 *   5. Hands off to `executeTool` from `@browseros/browser-mcp`'s tool
 *      framework. That handles arg validation, error formatting,
 *      tab-id metadata, and result composition.
 *
 * Known coarseness: the real catalogue's `act` tool covers every
 * mutation (click/type/fill/press/hover/scroll). We map it onto the
 * cockpit's `input` verb today, which means a site rule keyed on
 * `payments` does NOT clamp an `act({kind:'click'})` on a payment
 * button. Finer-grained classification (per-arg verb extraction) is
 * a follow-up.
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
import { check } from '../services/permissions'
import { persistScreenshot } from '../services/screenshots'
import { ensureAgentTabGroup } from '../services/tab-group-ops'
import { cancellationErrorResult } from './cancellation-result'
import { asRegister, type ToolResult } from './register-fn'

/**
 * Schemes the cockpit refuses to forward to `navigate`, regardless of
 * what the parent server's tool schema would accept. The real navigate
 * tool's zod input is `z.string().optional()` with no scheme check, so
 * without this guard a `javascript:`, `file:`, or `data:` URL would
 * pass the permission gate and reach the CDP layer. Re-asserts the
 * defense the old per-tool wrapper had before we switched to the real
 * catalogue.
 */
const NAVIGATE_BLOCKED_SCHEMES = new Set(['javascript:', 'file:', 'data:'])

/**
 * Maps each tool in the real catalogue to a permission verb. Tools
 * that mutate site context (`tabs`, `navigate`, `windows`,
 * `tab_groups`) map to `navigate`; `upload` maps to the catalog's
 * own `upload` verb; everything else maps to `input`, the cockpit's
 * catch-all for "click / type / read / etc.".
 *
 * `act`, `run`, `evaluate`, and `download` are deliberately lumped
 * under `input` despite being the higher-risk tools in the surface:
 * `download` has no dedicated catalog verb yet, and `run` /
 * `evaluate` execute arbitrary JS in page context. A richer
 * classifier (look at the `kind` arg of `act`, block `run` /
 * `evaluate` unless the agent opts in, add a `download` verb) is
 * the follow-up that closes this gap. Dispatches of the
 * arbitrary-script tools are logged for audit until that lands.
 */
const TOOL_TO_VERB: Record<string, string> = {
  tabs: 'navigate',
  navigate: 'navigate',
  windows: 'navigate',
  tab_groups: 'navigate',
  upload: 'upload',
  snapshot: 'input',
  diff: 'input',
  act: 'input',
  read: 'input',
  grep: 'input',
  screenshot: 'input',
  wait: 'input',
  pdf: 'input',
  download: 'input',
  run: 'input',
  evaluate: 'input',
}

const ARBITRARY_SCRIPT_TOOLS = new Set(['run', 'evaluate'])

/**
 * Picks a domain for the permission check. `navigate` carries the
 * target URL in its args, which is the cleanest signal we have until
 * per-page URL tracking ships. Every other tool falls back to the
 * agent's first declared site, or `'*'` so wildcard site rules still
 * fire for agents with an empty `selectedSites`.
 */
function domainForCall(
  toolName: string,
  rawArgs: unknown,
  agent: StoredAgentProfile,
): string {
  if (
    toolName === 'navigate' &&
    typeof rawArgs === 'object' &&
    rawArgs !== null
  ) {
    const url = (rawArgs as { url?: unknown }).url
    if (typeof url === 'string' && url.length > 0) {
      try {
        const hostname = new URL(url).hostname
        if (hostname) return hostname
      } catch {
        // fall through to the agent hint
      }
    }
  }
  return agent.selectedSites[0] ?? '*'
}

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
    const verb = TOOL_TO_VERB[tool.name] ?? 'input'
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
        const domain = domainForCall(tool.name, rawArgs, agent)
        const verdict = await check({
          agentId: agent.id,
          verb,
          domain,
        })
        if (verdict.verdict === 'block') {
          return {
            content: [
              {
                type: 'text',
                text: `blocked by ${verdict.source}: ${tool.name} on ${domain}`,
              },
            ],
            isError: true,
          } satisfies ToolResult
        }
        if (verdict.verdict === 'ask') {
          return {
            content: [
              {
                type: 'text',
                text: `approval required for ${tool.name} on ${domain}; the cockpit will surface this once run-lifecycle approvals ship`,
              },
            ],
            isError: true,
          } satisfies ToolResult
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
          // `run` and `evaluate` execute arbitrary JS in the page's
          // context. They map to the same `input` verb as low-risk
          // reads today, so an agent with `input: 'Auto'` runs
          // scripts without confirmation. A dedicated catalog verb
          // (and a UI surface for it) is the proper fix; this log
          // keeps the dispatch auditable until that lands.
          logger.warn('cockpit dispatched arbitrary-script tool', {
            tool: tool.name,
            agentId: agent.id,
            domain,
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
 * Rewrite a successful `tabs list` result to only include pages the
 * calling agent owns. Both channels are rebuilt from the surviving
 * subset:
 *   - `structuredContent.pages` is filtered.
 *   - `content[0].text` is rebuilt via the tool's `formatPageLine`
 *     shape (`[N] URL (title)` or `[N] URL` when no title). Empty
 *     survivors yield `(no open pages)` which matches the underlying
 *     tool's empty output so codex's LLM interprets it as "no tabs,
 *     open one" and dispatches `tabs new` instead of hijacking.
 *
 * Exported for unit tests; production callers reach it via the
 * dispatch handler.
 */
export function filterTabsListToAgent<
  R extends {
    content: unknown
    isError?: boolean
    structuredContent?: unknown
  },
>(result: R, owned: ReadonlySet<number>): R {
  const sc = result.structuredContent as
    | { pages?: Array<{ page: number; url?: string; title?: string }> }
    | undefined
  const allPages = sc?.pages ?? []
  const surviving = allPages.filter((p) => owned.has(p.page))
  const lines = surviving.map(
    (p) => `[${p.page}] ${p.url ?? ''}${p.title ? ` (${p.title})` : ''}`,
  )
  const text = lines.length > 0 ? lines.join('\n') : '(no open pages)'
  return {
    ...result,
    isError: false,
    content: [{ type: 'text', text }],
    structuredContent: { pages: surviving },
  } as R
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
 * v2 deliberately skips the per-agent permission gate: there is no
 * `StoredAgentProfile` to look up. The navigate-scheme guard stays
 * (it is a hard security check on the URL shape, not a per-agent
 * policy). A future "global permissions" surface can grow back into
 * this code path when product needs it.
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
                // Filter the list to only pages this agent owns.
                // With no owned pages, the surviving text is
                // `(no open pages)` which mirrors the tool's own
                // empty output; the LLM interprets it as "I have no
                // tabs, I need to open one" and calls `tabs new`.
                const { agentId } = agentIdentityFromClient(identity)
                result = filterTabsListToAgent(
                  result,
                  agentTabs.ownedBy(agentId),
                )
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
