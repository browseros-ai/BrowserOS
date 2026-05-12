/**
 * Site activity extractor for BrowserOS VCC compaction.
 *
 * Extracts which sites were visited and what tools were used per site.
 * Produces a timeline of site navigation with tool usage counts.
 */

import { extractUrl, isBrowserTool, isNavigationTool } from '../tool-args'
import type { NormalizedBlock, SiteVisit } from '../types'

const MAX_SITES = 10

/** Extract the domain (hostname) from a URL string. */
const extractDomain = (url: string): string => {
  try {
    return new URL(url).hostname
  } catch {
    // Malformed URL — return first 40 chars
    return url.slice(0, 40)
  }
}

export const extractActivities = (blocks: NormalizedBlock[]): SiteVisit[] => {
  const visits: SiteVisit[] = []
  let currentVisit: SiteVisit | null = null
  let order = 0

  const incrementTool = (visit: SiteVisit, toolName: string) => {
    visit.tools.set(toolName, (visit.tools.get(toolName) ?? 0) + 1)
  }

  for (const block of blocks) {
    if (block.kind !== 'tool_call') continue

    const url = extractUrl(block.args)

    // Navigation tool with URL → new or return visit
    if (isNavigationTool(block.name) && url) {
      const domain = extractDomain(url)

      // Check if we already have a visit to this domain
      const existing = visits.find((v) => v.domain === domain)

      if (existing) {
        // Return visit — reuse existing, update current
        currentVisit = existing
        incrementTool(existing, block.name)
      } else {
        // New site
        order++
        const visit: SiteVisit = {
          url,
          domain,
          order,
          sourceIndex: block.sourceIndex ?? 0,
          tools: new Map([[block.name, 1]]),
        }
        visits.push(visit)
        currentVisit = visit
      }
      continue
    }

    // Any browser tool → associate with current visit
    if (currentVisit && isBrowserTool(block.name)) {
      incrementTool(currentVisit, block.name)
    }
  }

  // Cap at MAX_SITES — keep the most recent
  if (visits.length > MAX_SITES) {
    return visits.slice(-MAX_SITES)
  }

  return visits
}
