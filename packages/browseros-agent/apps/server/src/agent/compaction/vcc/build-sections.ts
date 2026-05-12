/**
 * Section builder — vendored from pi-vcc core/build-sections.ts.
 * Orchestrates all extractors into SectionData.
 */

import {
  buildBriefSections,
  sectionsToTranscript,
  stringifyBrief,
} from './brief'
import { clipSentence, firstLine, nonEmptyLines } from './content'
import { extractActivities } from './extract/activities'
import { extractCommits, formatCommits } from './extract/commits'
import { extractFiles } from './extract/files'
import { extractGoals } from './extract/goals'
import {
  dedupPreferencesAgainstGoals,
  extractPreferences,
} from './extract/preferences'
import type { NormalizedBlock, SectionData } from './types'

export interface BuildSectionsInput {
  blocks: NormalizedBlock[]
}

const BLOCKER_RE =
  /\b(fail(ed|s|ure|ing)?|broken|cannot|can't|won't work|does not work|doesn't work|still (broken|failing|wrong)|blocked|blocker|not (fixed|resolved|working)|crash(es|ed|ing)?)\b/i

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: vendored pi-vcc extraction logic
const extractOutstandingContext = (blocks: NormalizedBlock[]): string[] => {
  const items: string[] = []
  const tail = blocks.slice(-20)

  for (const b of tail) {
    if (b.kind === 'tool_result' && b.isError) {
      items.push(`[${b.name}] ${firstLine(b.text, 150)}`)
      continue
    }

    if (b.kind === 'assistant' || b.kind === 'user') {
      for (const line of nonEmptyLines(b.text)) {
        if (!BLOCKER_RE.test(line)) continue
        if (line.length < 15) continue
        if (/^\s*[-*+>]\s/.test(line)) continue
        if (/^\s*\(/.test(line)) continue
        if (!/^\s*["'`*_]?[A-Z`]/.test(line)) continue
        const clipped =
          b.kind === 'user'
            ? `[user] ${clipSentence(line, 150)}`
            : clipSentence(line, 150)
        if (!items.includes(clipped)) items.push(clipped)
        break
      }
    }
  }

  return items.slice(0, 5)
}

const formatFileActivity = (blocks: NormalizedBlock[]): string[] => {
  const act = extractFiles(blocks)
  for (const p of act.modified) act.created.delete(p)
  const lines: string[] = []
  const cap = (set: Set<string>, limit: number) => {
    const arr = [...set]
    if (arr.length <= limit) return arr.join(', ')
    return `${arr.slice(0, limit).join(', ')} (+${arr.length - limit} more)`
  }
  if (act.modified.size > 0) lines.push(`Modified: ${cap(act.modified, 10)}`)
  if (act.created.size > 0) lines.push(`Created: ${cap(act.created, 10)}`)
  if (act.read.size > 0) lines.push(`Read: ${cap(act.read, 10)}`)
  return lines
}

export const buildSections = (input: BuildSectionsInput): SectionData => {
  const { blocks } = input
  const briefSections = buildBriefSections(blocks)
  const sessionGoal = extractGoals(blocks)
  const userPreferences = dedupPreferencesAgainstGoals(
    extractPreferences(blocks),
    sessionGoal,
  )
  return {
    sessionGoal,
    outstandingContext: extractOutstandingContext(blocks),
    filesAndChanges: formatFileActivity(blocks),
    commits: formatCommits(extractCommits(blocks)),
    ...(() => {
      const { visits, timeline } = extractActivities(blocks)
      return { siteActivity: visits, timelineEvents: timeline }
    })(),
    userPreferences,
    briefTranscript: stringifyBrief(briefSections),
    transcriptEntries: sectionsToTranscript(briefSections),
  }
}
