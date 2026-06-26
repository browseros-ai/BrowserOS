/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Shared shape for the structured error result the cockpit returns
 * when an operator cancels an in-flight tool dispatch via the Stop
 * button. The MCP client (the agent's harness) sees this as a
 * normal `isError: true` tool result with a text-content explanation
 * and a structured discriminator the harness can detect if it wants
 * to special-case cancellations vs other errors.
 */

import type { ToolResult } from './register-fn'

/**
 * The discriminator surfaced on `structuredContent` so a harness can
 * tell operator-driven cancellations apart from other tool errors
 * (timeouts, target detached, etc).
 */
export const CANCELLATION_DISCRIMINATOR = 'cockpit.operator-cancelled' as const

export interface CancellationStructured {
  cancellationReason: string
  cancellationKind: typeof CANCELLATION_DISCRIMINATOR
}

export function cancellationErrorResult(reason: string): ToolResult {
  return {
    isError: true,
    content: [{ type: 'text', text: reason }],
    structuredContent: {
      cancellationReason: reason,
      cancellationKind: CANCELLATION_DISCRIMINATOR,
    } satisfies CancellationStructured,
  }
}
