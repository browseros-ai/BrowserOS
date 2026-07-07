/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tri-bucket ownership annotation for the `tabs list` MCP result.
 *
 * Every open page is classified as one of:
 *   - `mine`         owner === caller (this session)
 *   - `user`         no agent owner recorded (operator-opened tab)
 *   - `other-agent`  owner is a different session's agentId
 *
 * The classifier keeps every page in the response (no filtering) and
 * decorates the text + structuredContent channels so the LLM can
 * distinguish ownership at a glance. Empty buckets are omitted from
 * the text output; a fully empty list still renders `(no open pages)`
 * for backwards compatibility with the pre-refactor empty-state cue.
 *
 * The cross-agent page guard in register.ts still hard-rejects a
 * dispatch on a foreign page. This annotator only affects visibility.
 */

import {
  agentIdentityFromClient,
  type ClientIdentity,
  type IdentityService,
} from '../lib/mcp-session'

export type TabOwnership = 'mine' | 'user' | 'other-agent'

export interface StructuredPage {
  page: number
  url?: string
  title?: string
}

export interface AnnotatedPage extends StructuredPage {
  ownership: TabOwnership
  ownerAgentId: string | null
  ownerLabel: string | null
}

export interface OwnershipDeps {
  callerAgentId: string
  resolveOwner: (pageId: number) => string | null
  labelForAgentId: (agentId: string) => string | null
}

export function classify(
  page: StructuredPage,
  deps: OwnershipDeps,
): AnnotatedPage {
  const owner = deps.resolveOwner(page.page)
  if (owner === null) {
    return {
      ...page,
      ownership: 'user',
      ownerAgentId: null,
      ownerLabel: null,
    }
  }
  if (owner === deps.callerAgentId) {
    return {
      ...page,
      ownership: 'mine',
      ownerAgentId: owner,
      ownerLabel: null,
    }
  }
  return {
    ...page,
    ownership: 'other-agent',
    ownerAgentId: owner,
    ownerLabel: deps.labelForAgentId(owner),
  }
}

interface TabsListResult {
  content: unknown
  isError?: boolean
  structuredContent?: unknown
}

export function annotateTabsListWithOwnership<R extends TabsListResult>(
  result: R,
  deps: OwnershipDeps,
): R {
  const sc = result.structuredContent as
    | { pages?: StructuredPage[] }
    | undefined
  const raw = sc?.pages ?? []
  const annotated = raw.map((p) => classify(p, deps))
  return {
    ...result,
    isError: false,
    content: [{ type: 'text', text: renderText(annotated) }],
    structuredContent: { pages: annotated },
  } as R
}

function renderText(pages: AnnotatedPage[]): string {
  if (pages.length === 0) return '(no open pages)'
  const mine = pages.filter((p) => p.ownership === 'mine')
  const user = pages.filter((p) => p.ownership === 'user')
  const other = pages.filter((p) => p.ownership === 'other-agent')
  const sections: string[] = []
  if (mine.length > 0) sections.push(section('Your tabs:', mine))
  if (user.length > 0) sections.push(section("User's tabs:", user))
  if (other.length > 0) sections.push(section("Other agents' tabs:", other))
  return sections.join('\n\n')
}

function section(header: string, pages: AnnotatedPage[]): string {
  return `${header}\n${pages.map(formatLine).join('\n')}`
}

function formatLine(p: AnnotatedPage): string {
  const title = p.title ? ` (${p.title})` : ''
  const suffix =
    p.ownership === 'other-agent' && p.ownerLabel
      ? `, owned by ${p.ownerLabel}`
      : ''
  return `[${p.page}] ${p.url ?? ''}${title}${suffix}`
}

/**
 * Builds an OwnershipDeps for a given caller identity. The label
 * cache snapshots `identityService.list()` at call time; an agent
 * whose session already closed will have no entry and `labelForAgentId`
 * returns null (the text render then omits the "owned by ..." suffix).
 */
export function buildOwnershipDeps(
  callerIdentity: ClientIdentity,
  agentTabs: { resolveOwner: (pageId: number) => string | null },
  identityService: IdentityService,
): OwnershipDeps {
  const { agentId: callerAgentId } = agentIdentityFromClient(callerIdentity)
  const labelCache = new Map<string, string>()
  for (const id of identityService.list()) {
    const { agentId, slug } = agentIdentityFromClient(id)
    const label = slug || id.clientName || 'unknown'
    labelCache.set(agentId, label)
  }
  return {
    callerAgentId,
    resolveOwner: (pageId) => agentTabs.resolveOwner(pageId),
    labelForAgentId: (agentId) => labelCache.get(agentId) ?? null,
  }
}
