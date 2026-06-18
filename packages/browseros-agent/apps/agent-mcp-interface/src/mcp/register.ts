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
import type { StoredAgentProfile } from '../routes/agents/schemas'
import { check } from '../services/permissions'
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
 * Maps each tool in the real catalogue to a permission verb. `tabs`
 * and `navigate` mutate site context so they map to `navigate`;
 * every other tool maps to `input`, the cockpit's catch-all for
 * "click / type / read / etc.".
 *
 * `act` and `run` are intentionally lumped under `input` despite
 * being the highest-risk tools. A richer classifier (look at the
 * `kind` arg of `act`, or block `run` unless the agent opts in) is
 * the follow-up that closes this gap.
 */
const TOOL_TO_VERB: Record<string, string> = {
  tabs: 'navigate',
  navigate: 'navigate',
  snapshot: 'input',
  diff: 'input',
  act: 'input',
  read: 'input',
  grep: 'input',
  screenshot: 'input',
  wait: 'input',
  run: 'input',
}

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

        if (tool.name === 'run') {
          // `run` executes arbitrary JS in the page's context. It maps
          // to the same `input` verb as low-risk reads today, so an
          // agent with `input: 'Auto'` runs scripts without
          // confirmation. A dedicated `run` verb in the catalog (and
          // a UI surface for it) is the proper fix; this log keeps the
          // dispatch auditable until that lands. See PR review.
          logger.warn('cockpit dispatched run (arbitrary script)', {
            agentId: agent.id,
            domain,
          })
        }
        const result = await executeTool(tool, rawArgs, {
          session,
          signal: extra?.signal,
        })
        return {
          content: result.content as ToolResult['content'],
          isError: result.isError,
          structuredContent: result.structuredContent,
        }
      },
    )
  }
}
