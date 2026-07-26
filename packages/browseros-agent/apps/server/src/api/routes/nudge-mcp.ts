/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Dedicated in-process MCP server exposing a single tool:
 * `suggest_app_connection`. The ACPX runtime adds this URL as a
 * second mcpServers entry (alongside the browser-actions /mcp entry)
 * with X-BrowserOS-Agent-Id + X-BrowserOS-Session-Id headers
 * identifying the active turn.
 *
 * Direct in-process emission via TurnRegistry.pushEvent is required
 * because acpx-ai-provider collapses MCP tool-result content to a
 * status string at the AI SDK boundary — the structured JSON payload
 * cannot be recovered downstream. So the tool returns a short
 * STOP-and-wait directive to the host LLM, and the card itself is
 * delivered out-of-band via an `app_connection_request` event on the
 * in-flight turn's stream.
 *
 * Mirrors agent-company's apps/desktop/src/main/routes/nudge-mcp.ts
 * architecturally; the BrowserOS-side substitutions are (1) using the
 * existing TurnRegistry as the rendezvous instead of a DB-backed
 * EventSink, and (2) keying by (agentId, sessionId) instead of a
 * thread id.
 */

import { randomUUID } from 'node:crypto'
import { StreamableHTTPTransport } from '@hono/mcp'
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { Hono } from 'hono'
import { type ZodRawShape, z } from 'zod'
import { MAIN_AGENT_SESSION_ID } from '../../lib/agents/agent-types'
import type { TurnRegistry } from '../../lib/agents/turns/active-turn-registry'
import { logger } from '../../lib/logger'

// The MCP SDK's registerTool is overloaded heavily enough to trigger
// TS2589 "excessively deep" on this small surface. The browser tools
// at @browseros/browser-mcp/register re-type the same call signature
// for the same reason; we follow the pattern.
type RegisterToolFn = (
  name: string,
  config: { description: string; inputSchema?: ZodRawShape },
  handler: (args: Record<string, unknown>) => Promise<{
    content: Array<{ type: 'text'; text: string }>
    isError?: boolean
  }>,
) => void

export const NUDGE_AGENT_ID_HEADER = 'X-BrowserOS-Agent-Id'
export const NUDGE_SESSION_ID_HEADER = 'X-BrowserOS-Session-Id'

const NUDGE_DESCRIPTION = [
  'Call this tool whenever the user asks for a task that needs a third-party',
  'service (Linear, Gmail, Slack, GitHub, Notion, Jira, Figma, Salesforce,',
  'Google Calendar/Docs/Drive/Sheets, LinkedIn, Airtable, Confluence,',
  'HubSpot, Stripe, PostHog, Mixpanel, Discord, Cal.com, Resend, Zendesk,',
  'Intercom, Asana, ClickUp, Monday, Microsoft Teams, Outlook Mail/Calendar,',
  'Supabase, Vercel, Postman, Cloudflare, Brave Search, Mem0, Dropbox,',
  'OneDrive, WordPress, YouTube, Box, WhatsApp, Shopify, Google Forms) and',
  'either:',
  '  (a) a connector check reports the service as not connected, OR',
  '  (b) a tool call against that service returns 401 / Unauthorized, OR',
  '  (c) any response surfaces an authUrl / apiKeyUrl / "authorize here" link.',
  '',
  'CRITICAL output rules:',
  '  - Your response must contain ONLY this tool call.',
  '  - Do NOT include any text before or after the tool call.',
  '  - Do NOT paste the auth URL into your reply. The UI renders an',
  '    interactive connect card from this tool call. Pasting the URL',
  '    in text would duplicate the prompt and confuse the user.',
  '',
  'After this tool returns, STOP and wait for the user. They will reply',
  '"I\'ve connected X, continue..." once authorization completes. At that',
  'point retry the original tool call.',
].join('\n')

const inputSchema = {
  appName: z
    .string()
    .min(1)
    .describe(
      'The display name of the toolkit to connect, e.g. "Linear", "Gmail", "Slack". Match the casing of the BrowserOS catalog (proper-case, space-separated).',
    ),
  reason: z
    .string()
    .min(1)
    .describe(
      'A short user-facing rationale for the connection request, e.g. "to read your Linear issues" or "to send the Slack message you asked for". One sentence.',
    ),
}

export interface NudgeMcpRouteDeps {
  turnRegistry: TurnRegistry
}

export function createNudgeMcpRoute(deps: NudgeMcpRouteDeps) {
  const { turnRegistry } = deps

  return new Hono().post('/', async (c) => {
    const agentId = c.req.header(NUDGE_AGENT_ID_HEADER) ?? null
    const sessionIdHeader = c.req.header(NUDGE_SESSION_ID_HEADER) ?? null
    const sessionId = sessionIdHeader ?? MAIN_AGENT_SESSION_ID

    // Per-request McpServer + transport; matches the shared /mcp
    // route's request-scoped pattern.
    const server = new McpServer(
      { name: 'browseros-nudge', version: '0.0.1' },
      {
        instructions:
          'Single-tool MCP server. Calling suggest_app_connection renders an interactive connect card to the user; STOP and wait for their reply afterwards.',
      },
    )

    const registerTool = server.registerTool.bind(
      server,
    ) as unknown as RegisterToolFn
    registerTool(
      'suggest_app_connection',
      { description: NUDGE_DESCRIPTION, inputSchema },
      async (args) => {
        const typedArgs = args as { appName: string; reason: string }
        if (!agentId) {
          logger.warn('nudge tool called without agent id header', {
            sessionId,
          })
          return {
            isError: true,
            content: [
              {
                type: 'text' as const,
                text: 'Nudge tool called without an X-BrowserOS-Agent-Id header. The connect card cannot be attached to the current conversation.',
              },
            ],
          }
        }

        const turn = turnRegistry.getActiveFor(agentId, sessionId)
        if (!turn) {
          logger.warn('nudge tool called with no active turn', {
            agentId,
            sessionId,
          })
          return {
            isError: true,
            content: [
              {
                type: 'text' as const,
                text: 'No in-flight turn found for this conversation. The connect card cannot be attached.',
              },
            ],
          }
        }

        const pushed = turnRegistry.pushEvent(turn.turnId, {
          type: 'app_connection_request',
          // Fresh id so the renderer dedups on this card's identity
          // rather than the upstream acpx tool-call id (which the
          // handler does not see at the MCP transport layer).
          toolCallId: randomUUID(),
          appName: typedArgs.appName,
          reason: typedArgs.reason,
        })

        // pushEvent returns null when the turn ended between
        // getActiveFor and pushEvent (status flipped to terminal).
        // Without surfacing that the LLM would honour the STOP-and-wait
        // directive below and deadlock waiting for a reply to a card
        // the user will never see.
        if (!pushed) {
          logger.warn('nudge tool: event dropped (turn ended mid-call)', {
            agentId,
            sessionId,
          })
          return {
            isError: true,
            content: [
              {
                type: 'text' as const,
                text: 'The conversation turn ended before the connect card could be delivered. Ask the user to repeat their request.',
              },
            ],
          }
        }

        return {
          content: [
            {
              type: 'text' as const,
              text: `Connect card for ${typedArgs.appName} shown to the user. STOP. Do not produce any further text. Wait for the user's next message before continuing.`,
            },
          ],
        }
      },
    )

    const transport = new StreamableHTTPTransport({
      sessionIdGenerator: undefined,
      enableJsonResponse: true,
    })

    try {
      await server.connect(transport)
      return (await transport.handleRequest(c)) ?? c.body(null, 204)
    } catch (error) {
      logger.warn('nudge MCP route failed', {
        agentId,
        sessionId,
        error: error instanceof Error ? error.message : String(error),
      })
      return c.json(
        {
          message: error instanceof Error ? error.message : String(error),
        },
        500,
      )
    }
  })
}
