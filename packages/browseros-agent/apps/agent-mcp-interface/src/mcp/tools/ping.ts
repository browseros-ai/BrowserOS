/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Smoke tool used by the Phase 2 spike to prove the Bun + Hono +
 * MCP SDK triple composes end-to-end before we invest in real tools.
 * The Phase 3 commit swaps this for the navigate tool; the file will
 * disappear once `navigate` lands.
 */

import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { z } from 'zod'
import type { StoredAgentProfile } from '../../routes/agents/schemas'
import { asRegister } from '../register-fn'

export function registerSmokeTool(
  server: McpServer,
  agent: Pick<StoredAgentProfile, 'name' | 'slug'>,
): void {
  const register = asRegister(server)
  register(
    'ping',
    {
      description:
        'Echoes the agent name back to confirm the MCP route is live.',
      inputSchema: { message: z.string().optional() },
    },
    async (args) => {
      const message =
        typeof args.message === 'string' ? args.message : undefined
      return {
        content: [
          {
            type: 'text' as const,
            text: `pong from ${agent.name} (${agent.slug})${message ? `: ${message}` : ''}`,
          },
        ],
      }
    },
  )
}
