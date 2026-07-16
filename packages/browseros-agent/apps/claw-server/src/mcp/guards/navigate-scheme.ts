/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { logger } from '../../lib/logger'
import type { ToolCall, ToolGuard } from '../dispatch'

const NAVIGATE_BLOCKED_SCHEMES = new Set(['javascript:', 'file:', 'data:'])

/** Rejects navigate schemes that must never reach the CDP layer. */
export const guardNavigateScheme: ToolGuard = (call: ToolCall) => {
  if (call.tool.name !== 'navigate') return null
  const url = (call.args as { url?: unknown } | null | undefined)?.url
  if (typeof url !== 'string' || url.length === 0) return null

  const trimmedUrl = url.trim()
  const scheme = trimmedUrl.slice(0, trimmedUrl.indexOf(':') + 1).toLowerCase()
  if (!NAVIGATE_BLOCKED_SCHEMES.has(scheme)) return null

  logger.warn('cockpit tool dispatch rejected', {
    tool: call.tool.name,
    sessionId: call.sessionId || undefined,
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
  }
}
