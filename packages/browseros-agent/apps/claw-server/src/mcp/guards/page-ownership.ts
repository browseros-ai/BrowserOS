/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { ownershipStore } from '../../domain/ownership'
import { logger } from '../../lib/logger'
import { TOOLS_WITH_PAGE } from '../../lib/tab-activity'
import type { ToolGuard } from '../dispatch'

/** Prevents an identified agent from dispatching against another agent's page. */
export const guardPageOwnership: ToolGuard = (call) => {
  if (!TOOLS_WITH_PAGE.has(call.tool.name)) return null
  const page = (call.args as { page?: unknown } | null | undefined)?.page
  if (
    typeof page !== 'number' ||
    !Number.isInteger(page) ||
    page < 1 ||
    !call.agent ||
    !call.key
  ) {
    return null
  }

  // Cross-agent page guard. Reject dispatches whose `page` arg points at a
  // tab this agent does not own so an agent can only touch tabs it opened via
  // `tabs new`. This fires before executeTool; unknown identities fail open.
  if (ownershipStore.pagesOf(call.key).has(page)) return null

  logger.warn('cockpit v2 rejected foreign-page dispatch', {
    tool: call.tool.name,
    sessionId: call.sessionId || undefined,
    key: call.key,
    agentId: call.agent.agentId,
    page,
  })
  return {
    content: [
      {
        type: 'text',
        text: `page ${page} is not owned by this agent; call \`tabs new\` to open a fresh page and use the returned page id.`,
      },
    ],
    isError: true,
  }
}
