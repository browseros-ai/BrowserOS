/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Wires every browser tool from `@browseros/server`'s catalogue onto
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
 *   5. Hands off to `executeTool` from `@browseros/server`'s tool
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

import { executeTool } from '@browseros/server/tools/browser/framework'
import { BROWSER_TOOLS } from '@browseros/server/tools/browser/registry'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { ZodRawShape } from 'zod'
import { getBrowserSession } from '../lib/browser-session'
import { logger } from '../lib/logger'
import {
  agentIdentityFromClient,
  type ClientIdentity,
} from '../lib/mcp-session'
import { extractPageId, tabActivityRegistry } from '../lib/tab-activity'
import type { StoredAgentProfile } from '../routes/agents/schemas'
import { check } from '../services/permissions'
import { ensureAgentTabGroup } from '../services/tab-group-ops'
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
          content: result.content as ToolResult['content'],
          isError: result.isError,
          structuredContent: result.structuredContent,
        }
      },
    )
  }
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

        const result = await executeTool(tool, rawArgs, {
          session,
          signal: extra?.signal,
        })

        if (!result.isError) {
          const identity = resolveIdentity(extra?.sessionId)
          if (identity) {
            recordSuccessfulDispatchV2({
              toolName: tool.name,
              rawArgs,
              identity,
              session,
            })
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
                  void ensureAgentTabGroup({
                    agentId,
                    slug,
                    pageId,
                    session,
                    signal: extra?.signal,
                  })
                }
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
          content: result.content as ToolResult['content'],
          isError: result.isError,
          structuredContent: result.structuredContent,
        }
      },
    )
  }
}
