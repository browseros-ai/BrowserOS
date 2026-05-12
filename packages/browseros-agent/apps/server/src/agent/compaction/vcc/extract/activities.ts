import { extractUrl, isBrowserTool, isNavigationTool } from '../tool-args'
import type { NormalizedBlock, SiteVisit, TimelineEvent } from '../types'

const MAX_SITES = 10

/** Extract the domain from a URL string */
const extractDomain = (url: string): string => {
  try {
    return new URL(url).hostname
  } catch {
    return url.slice(0, 40)
  }
}

/** Format a tool count map as compact string: "tool1*3, tool2*1" */
const formatToolSnapshot = (tools: Map<string, number>): string => {
  if (tools.size === 0) return ''
  return [...tools.entries()]
    .sort((a, b) => b[1] - a[1])
    .map(([name, count]) => `${name}*${count}`)
    .join(', ')
}

export const extractActivities = (
  blocks: NormalizedBlock[],
): { visits: SiteVisit[]; timeline: TimelineEvent[] } => {
  const visits: SiteVisit[] = []
  const timeline: TimelineEvent[] = []
  const domainOrder = new Map<string, number>()
  let order = 0
  let currentVisit: SiteVisit | null = null
  const seenDomains = new Set<string>()

  for (const block of blocks) {
    if (block.kind !== 'tool_call') continue

    const url = extractUrl(block.args)

    // Navigation tool with URL → new or return visit
    if (isNavigationTool(block.name) && url) {
      const domain = extractDomain(url)

      if (!domainOrder.has(domain)) {
        order++
        domainOrder.set(domain, order)
      }

      const existing = visits.find((v) => v.domain === domain)
      const isReturnVisit = seenDomains.has(domain)
      seenDomains.add(domain)

      if (existing) {
        // Return visit — update sourceIndex to most recent, merge tool
        existing.sourceIndex = block.sourceIndex ?? 0
        existing.tools.set(
          block.name,
          (existing.tools.get(block.name) ?? 0) + 1,
        )
        currentVisit = existing
      } else {
        // New site
        const visit: SiteVisit = {
          url,
          domain,
          order: domainOrder.get(domain) ?? 0,
          sourceIndex: block.sourceIndex ?? 0,
          tools: new Map([[block.name, 1]]),
        }
        visits.push(visit)
        currentVisit = visit
      }

      // Record timeline event
      timeline.push({
        domain,
        sourceIndex: block.sourceIndex ?? 0,
        isReturnVisit,
        toolSnapshot: formatToolSnapshot(currentVisit.tools),
      })
      continue
    }

    // Any browser tool on current visit → increment count
    if (currentVisit && block.name && isBrowserTool(block.name)) {
      currentVisit.tools.set(
        block.name,
        (currentVisit.tools.get(block.name) ?? 0) + 1,
      )
    }
  }

  // Cap at MAX_SITES — sort by most recent sourceIndex, keep the most active/recent
  if (visits.length > MAX_SITES) {
    // Sort by last activity (sourceIndex desc), keep top 10
    const sorted = [...visits].sort((a, b) => b.sourceIndex - a.sourceIndex)
    const kept = new Set(sorted.slice(0, MAX_SITES).map((v) => v.domain))
    const capped = visits.filter((v) => kept.has(v.domain))
    visits.length = 0
    visits.push(...capped)
  }

  return { visits, timeline }
}
