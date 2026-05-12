/**
 * VCC Adapter — entry point for BrowserOS compaction.
 *
 * Provides `vccCompile()` which takes AI SDK ModelMessage[], normalizes,
 * filters noise, builds structured sections, and formats a summary string.
 * Merges with previous summary when provided.
 */
import type { ModelMessage } from 'ai'
import { buildSections } from './vcc/build-sections'
import { filterNoise } from './vcc/filter-noise'
import { formatSummary } from './vcc/format'
import { normalizeFromAiSdk } from './vcc/normalize'
import type { VccOverrides } from './vcc/types'

const RECALL_NOTE_RE = /Use `vcc_recall` to search for prior work.*?(?:\n|$)/g

const CONTINUATION_SUFFIX = '\n\nContinue from where you left off.'

/** Strip recall note and continuation suffix from a previous summary so they don't compound. */
const stripRecallNote = (text: string): string =>
  text.replace(RECALL_NOTE_RE, '').replace(CONTINUATION_SUFFIX, '').trimEnd()

/** Extract section content from a formatted summary for merging. */
const mergePrevious = (previous: string, fresh: string): string => {
  // Simple merge strategy: put fresh summary first, append previous context
  // The buildSections already captures the latest state, so fresh is authoritative.
  // Keep previous as a collapsed "Previous Context" block at the end.
  // (prevSections was removed — merge strategy is whole-text based)

  // If previous is very short, just concatenate
  if (previous.length < 200) {
    return `${fresh}\n\n---\n\n[Previous Context]\n${previous}`
  }

  // For longer previous summaries, include the most recent lines (tail)
  // VCC format puts goals first (least important) and transcript last (most important)
  const condensed = previous.split('\n').slice(-20).join('\n')
  return `${fresh}\n\n---\n\n[Previous Context]\n${condensed}`
}

/**
 * Compile a structured VCC summary from AI SDK messages.
 *
 * @param messages - AI SDK ModelMessage array to summarize
 * @param existingSummary - Previous summary text to merge (null on first compaction)
 * @param overrides - Optional VCC config overrides
 * @returns Formatted summary string, or null if no meaningful content
 */
export function vccCompile(
  messages: ModelMessage[],
  existingSummary: string | null,
  overrides?: VccOverrides,
): string | null {
  if (!messages || messages.length === 0) return null

  const blocks = filterNoise(normalizeFromAiSdk(messages))
  if (blocks.length === 0) return null

  const data = buildSections({ blocks })
  const fresh = formatSummary(data, overrides)

  if (!fresh) return null

  if (existingSummary) {
    return mergePrevious(stripRecallNote(existingSummary), fresh)
  }

  return fresh
}
