/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { type AgentKey, agentKeyFromSlug } from '../../domain/agent-key'
import { generateFunName } from './fun-names'

export interface ClientIdentity {
  sessionId: string
  clientName: string
  clientVersion: string
  clientTitle: string | null
  slug: string
  key: AgentKey
  generatedLabel: string
  label: string
  renameNudgesLeft: number
  firstSeenAt: number
}

export interface RetainedIdentity {
  key: AgentKey
  endedAt: number
}

export interface IdentityService {
  registerInitialize(input: {
    sessionId: string
    clientInfo: {
      name?: string | undefined
      version?: string | undefined
      title?: string | undefined
    }
  }): ClientIdentity
  getIdentity(sessionId: string): ClientIdentity | null
  setLabel(sessionId: string, label: string): void
  takeRenameNudge(sessionId: string): boolean
  endSession(sessionId: string): ClientIdentity | null
  list(): ClientIdentity[]
  listRetained(): RetainedIdentity[]
  forgetRetained(key: AgentKey): void
  size(): number
  clear(): void
}

export interface IdentityServiceDeps {
  now?: () => number
  random?: () => number
}

const SLUG_MAX_LEN = 64
const SESSION_NAME_NUDGE_LIMIT = 5

export function createIdentityService(
  deps: IdentityServiceDeps = {},
): IdentityService {
  const records = new Map<string, ClientIdentity>()
  const retained = new Map<AgentKey, number>()
  const now = deps.now ?? (() => Date.now())
  const random = deps.random ?? Math.random

  function keyAvailable(key: AgentKey): boolean {
    if (retained.has(key)) return false
    return Array.from(records.values()).every((record) => record.key !== key)
  }

  return {
    registerInitialize(input) {
      const existing = records.get(input.sessionId)
      if (existing) return existing

      const clientName = input.clientInfo.name?.trim() ?? ''
      const slug = slugifyClientName(clientName) || 'agent'
      const generatedLabel = generateFunName({
        random,
        isAvailable(label) {
          return keyAvailable(agentKeyFromSlug(`${slug}-${label}`))
        },
      })
      const record: ClientIdentity = {
        sessionId: input.sessionId,
        clientName,
        clientVersion: input.clientInfo.version?.trim() ?? '',
        clientTitle: input.clientInfo.title?.trim() || null,
        slug,
        key: agentKeyFromSlug(`${slug}-${generatedLabel}`),
        generatedLabel,
        label: generatedLabel,
        renameNudgesLeft: SESSION_NAME_NUDGE_LIMIT,
        firstSeenAt: now(),
      }
      records.set(input.sessionId, record)
      return record
    },
    getIdentity(sessionId) {
      return records.get(sessionId) ?? null
    },
    setLabel(sessionId, label) {
      const record = records.get(sessionId)
      if (record) record.label = label
    },
    takeRenameNudge(sessionId) {
      const record = records.get(sessionId)
      if (
        !record ||
        record.label !== record.generatedLabel ||
        record.renameNudgesLeft === 0
      ) {
        return false
      }
      record.renameNudgesLeft -= 1
      return true
    },
    endSession(sessionId) {
      const record = records.get(sessionId)
      if (!record) return null
      records.delete(sessionId)
      retained.set(record.key, now())
      return record
    },
    list() {
      return Array.from(records.values())
    },
    listRetained() {
      return Array.from(retained, ([key, endedAt]) => ({ key, endedAt }))
    },
    forgetRetained(key) {
      retained.delete(key)
    },
    size() {
      return records.size
    },
    clear() {
      records.clear()
      retained.clear()
    },
  }
}

/** Lowercase alphanumeric + hyphen, trimmed and capped. */
export function slugifyClientName(raw: string): string {
  const cleaned = raw
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  if (cleaned.length === 0) return ''
  return cleaned.slice(0, SLUG_MAX_LEN)
}

/** Resolves the per-conversation key used for all ownership. */
export function agentKeyFromClient(identity: ClientIdentity): AgentKey {
  return identity.key
}

/** Bridges the born identity to registry and audit call sites. */
export function agentIdentityFromClient(identity: ClientIdentity): {
  agentId: string
  slug: string
} {
  return { agentId: identity.key, slug: identity.slug }
}
