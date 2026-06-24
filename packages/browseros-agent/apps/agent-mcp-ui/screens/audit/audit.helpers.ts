import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import { hexForSlug } from './audit.colors'

const NINE_SECONDS = 9_000
const ONE_MINUTE = 60_000
const ONE_HOUR = 3_600_000
const ONE_DAY = 86_400_000

export function formatRelative(createdAt: number, now: number): string {
  const delta = now - createdAt
  if (delta < NINE_SECONDS) return 'just now'
  if (delta < ONE_MINUTE) return `${Math.floor(delta / 1000)}s ago`
  if (delta < ONE_HOUR) return `${Math.floor(delta / ONE_MINUTE)}m ago`
  if (delta < ONE_DAY) return `${Math.floor(delta / ONE_HOUR)}h ago`
  return `${Math.floor(delta / ONE_DAY)}d ago`
}

export function siteOf(url: string | null): string {
  if (!url) return ''
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return url
  }
}

export interface AgentChip {
  agentId: string
  slug: string
  agentLabel: string
  color: string
  count: number
}

export function agentChipsFor(rows: ToolDispatchRow[]): AgentChip[] {
  const map = new Map<string, AgentChip>()
  for (const r of rows) {
    const existing = map.get(r.agentId)
    if (existing) {
      existing.count += 1
      continue
    }
    map.set(r.agentId, {
      agentId: r.agentId,
      slug: r.slug,
      agentLabel: r.agentLabel,
      color: hexForSlug(r.slug),
      count: 1,
    })
  }
  return Array.from(map.values()).sort((a, b) => b.count - a.count)
}

export function parseResultMeta(raw: string | null): {
  isError: boolean
  contentSummary: string
  structuredKeys: string[]
} | null {
  if (!raw) return null
  try {
    const v = JSON.parse(raw) as {
      isError?: boolean
      contentSummary?: string
      structuredKeys?: string[]
    }
    return {
      isError: Boolean(v.isError),
      contentSummary: v.contentSummary ?? 'unknown',
      structuredKeys: Array.isArray(v.structuredKeys) ? v.structuredKeys : [],
    }
  } catch {
    return null
  }
}
