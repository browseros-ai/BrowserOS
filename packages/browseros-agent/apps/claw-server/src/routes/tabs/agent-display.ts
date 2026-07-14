/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure resolver that converts a `tabActivityRegistry` record's
 * `(agentId, slug)` pair into the display fields the homepage card
 * consumes. The fallback is:
 *
 *   1. the registry's session-scoped `agentId` matches a
 *      live identity record. Use the identity's `clientTitle ??
 *      clientName` as the label, and the hex matching the agent's
 *      BrowserOS tab group colour so the homepage card's left border
 *      visually matches the tab strip.
 *   2. the identity is gone (e.g. session closed before the
 *      homepage polled). Use the slug itself, harness null, colour
 *      derived from the slug for stability.
 */

import { hexForSlug } from '../../lib/agent-tab-groups'
import type { ClientIdentity } from '../../lib/mcp-session'

export interface AgentDisplay {
  agentLabel: string
  harness: string | null
  color: string | null
}

export function resolveAgentDisplay(
  agentId: string,
  slug: string,
  identitiesByAgentId: ReadonlyMap<string, ClientIdentity>,
): AgentDisplay {
  const identity = identitiesByAgentId.get(agentId)
  if (identity) {
    const label =
      identity.clientTitle && identity.clientTitle.length > 0
        ? identity.clientTitle
        : identity.clientName.length > 0
          ? identity.clientName
          : slug
    return {
      agentLabel: label,
      harness: null,
      color: hexForSlug(slug),
    }
  }
  return {
    agentLabel: slug,
    harness: null,
    color: hexForSlug(slug),
  }
}
