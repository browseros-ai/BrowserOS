/**
 * Vendored from pi-vcc core/format.ts — format-agnostic.
 * Formats SectionData into a summary string.
 */
import type { SectionData, SiteVisit, VccOverrides } from './types'

const DEFAULT_MAX_TRANSCRIPT_LINES = 120

const section = (title: string, items: string[]): string => {
  if (items.length === 0) return ''
  const body = items.map((i) => `- ${i}`).join('\n')
  return `[${title}]\n${body}`
}

export const capBrief = (text: string, maxLines?: number): string => {
  const limit = maxLines ?? DEFAULT_MAX_TRANSCRIPT_LINES
  const lines = text.split('\n')
  if (lines.length <= limit) return text
  const omitted = lines.length - limit
  const kept = lines.slice(-limit)
  const firstHeader = kept.findIndex((l) => /^\[.+\]/.test(l))
  const clean = firstHeader > 0 ? kept.slice(firstHeader) : kept
  return `...(${omitted} earlier lines omitted)\n\n${clean.join('\n')}`
}

export const RECALL_NOTE =
  'Use `vcc_recall` to search for prior work, decisions, and context from before this summary. ' +
  'Do not redo work already completed.'

const formatSiteActivity = (visits: SiteVisit[]): string => {
  if (visits.length === 0) return ''

  const lines = visits.map((v) => {
    const tools = [...v.tools.entries()]
      .sort((a, b) => b[1] - a[1])
      .map(([name, count]) => `${name}*${count}`)
      .join(', ')
    return `${v.order}. ${v.domain} — ${tools}`
  })

  return `[Site Activity]\n${lines.map((l) => `- ${l}`).join('\n')}`
}

const formatTimeline = (visits: SiteVisit[]): string => {
  if (visits.length === 0) return ''

  const lines = visits.map((v) => {
    const totalTools = [...v.tools.values()].reduce((a, b) => a + b, 0)
    return `#${v.order} → ${v.domain} [tools: ${totalTools}]`
  })

  return `[Timeline]\n${lines.join('\n')}`
}

export const formatSummary = (
  data: SectionData,
  overrides?: VccOverrides,
): string => {
  const maxTranscript =
    overrides?.maxTranscriptLines ?? DEFAULT_MAX_TRANSCRIPT_LINES

  const headerParts = [
    section('Session Goal', data.sessionGoal),
    section('Files And Changes', data.filesAndChanges),
    section('Commits', data.commits),
    section('Outstanding Context', data.outstandingContext),
    formatSiteActivity(data.siteActivity),
    formatTimeline(data.siteActivity),
    section('User Preferences', data.userPreferences),
  ].filter(Boolean)

  const parts: string[] = []
  if (headerParts.length > 0) {
    parts.push(headerParts.join('\n\n'))
  }
  if (data.briefTranscript) {
    parts.push(capBrief(data.briefTranscript, maxTranscript))
  }

  if (parts.length === 0) return ''

  return parts.join('\n\n---\n\n')
}
