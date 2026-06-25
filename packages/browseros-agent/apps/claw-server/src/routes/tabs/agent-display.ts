/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure resolver that converts a `tabActivityRegistry` record's
 * `(agentId, slug)` pair into the display fields the homepage card
 * consumes. The three-way fallback is:
 *
 *   1. legacy path: the registry's `agentId` matches a stored agent
 *      profile (PR 2 behaviour). Use the profile's `name`, `harness`.
 *   2. v2 path: the registry's `agentId` matches a live identity
 *      record (a slugified `clientInfo.name`). Use the identity's
 *      `clientTitle ?? clientName` as the label, and the hex matching
 *      the agent's BrowserOS tab group colour so the homepage card's
 *      left border visually matches the tab strip.
 *   3. final: the identity is gone (e.g. session closed before the
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

export interface AgentProfileLike {
  id: string
  name: string
  harness: string
}

export interface AgentDisplayDeps {
  profilesById: ReadonlyMap<string, AgentProfileLike>
  identitiesByAgentId: ReadonlyMap<string, ClientIdentity>
}

export function resolveAgentDisplay(
  agentId: string,
  slug: string,
  deps: AgentDisplayDeps,
): AgentDisplay {
  const profile = deps.profilesById.get(agentId)
  if (profile) {
    return {
      agentLabel: profile.name,
      harness: profile.harness,
      color: null,
    }
  }
  const identity = deps.identitiesByAgentId.get(agentId)
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
