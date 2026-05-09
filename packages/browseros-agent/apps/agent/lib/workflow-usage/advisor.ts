import type {
  WorkflowSkillSuggestion,
  WorkflowUsageAnalysis,
  WorkflowUsageRecord,
} from './types'

export type WorkflowAdvisorCommand = 'analyze' | 'view' | 'clear'

const DEFAULT_MIN_RUNS = 2
const DEFAULT_SUGGESTION_LIMIT = 3
const MIN_PATTERN_LENGTH = 2
const MAX_PATTERN_LENGTH = 8

const PRIVACY_NOTE =
  'This analysis uses only local tool-name sequences. BrowserOS does not include URLs, page content, prompts, tool inputs, or tool outputs in this workflow pattern data.'

const TOOL_LABELS: Record<string, string> = {
  click: 'Click',
  extract_data: 'Extract data',
  filesystem_read: 'Read file',
  filesystem_write: 'Write file',
  get_page_content: 'Read page',
  new_page: 'Open page',
  navigate: 'Navigate',
  open: 'Open',
  screenshot: 'Screenshot',
  search: 'Search',
  type: 'Type',
}

function normalizeCommandText(text: string): string {
  return text.toLowerCase().replace(/\s+/g, ' ').trim()
}

export function detectWorkflowAdvisorCommand(
  text: string,
): WorkflowAdvisorCommand | null {
  const normalized = normalizeCommandText(text)
  if (!normalized) return null

  const mentionsWorkflowData =
    normalized.includes('workflow usage') ||
    normalized.includes('usage pattern') ||
    normalized.includes('workflow pattern') ||
    normalized.includes('skill suggestion')

  if (
    mentionsWorkflowData &&
    /\b(clear|delete|reset|forget)\b/.test(normalized)
  ) {
    return 'clear'
  }

  if (
    mentionsWorkflowData &&
    /\b(show|view|list|display|what)\b/.test(normalized)
  ) {
    return 'view'
  }

  if (
    normalized.includes('analyze my workflow') ||
    normalized.includes('analyse my workflow') ||
    normalized.includes('what patterns do you see') ||
    normalized.includes('suggest skills') ||
    normalized.includes('find skill suggestions') ||
    normalized.includes('what can be automated') ||
    normalized.includes('analyze workflow patterns') ||
    normalized.includes('analyse workflow patterns')
  ) {
    return 'analyze'
  }

  return null
}

function normalizeToolName(toolName: string): string {
  return toolName.trim().toLowerCase()
}

export function normalizeToolSequence(toolNames: string[]): string[] {
  const sequence: string[] = []
  for (const rawName of toolNames) {
    const toolName = normalizeToolName(rawName)
    if (!toolName) continue
    if (sequence[sequence.length - 1] === toolName) continue
    sequence.push(toolName)
  }
  return sequence.slice(0, MAX_PATTERN_LENGTH)
}

function labelTool(toolName: string): string {
  const normalized = normalizeToolName(toolName)
  const known = TOOL_LABELS[normalized]
  if (known) return known

  return normalized
    .replace(/^(browser|tool|mcp)[_-]/, '')
    .replace(/[_-]+/g, ' ')
    .replace(/\b\w/g, (char) => char.toUpperCase())
}

function titleCase(value: string): string {
  return value.replace(/\b\w/g, (char) => char.toUpperCase())
}

function buildSuggestionTitle(pattern: string[]): string {
  const labels = pattern.map((toolName) => titleCase(labelTool(toolName)))
  const uniqueLabels = Array.from(new Set(labels))
  if (uniqueLabels.length <= 2) return `${uniqueLabels.join(' + ')} Skill`
  return `${uniqueLabels[0]} to ${uniqueLabels.at(-1)} Skill`
}

function buildBenefit(pattern: string[]): string {
  const patternLabel = pattern.map(labelTool).join(' -> ')
  return `Turn the repeated ${patternLabel} sequence into reusable skill instructions.`
}

function compareSuggestions(
  left: WorkflowSkillSuggestion,
  right: WorkflowSkillSuggestion,
): number {
  return (
    right.runCount - left.runCount ||
    right.pattern.length - left.pattern.length ||
    right.lastUsedAt - left.lastUsedAt
  )
}

export function analyzeWorkflowUsage(
  records: WorkflowUsageRecord[],
  options?: { minRuns?: number; limit?: number },
): WorkflowUsageAnalysis {
  const minRuns = options?.minRuns ?? DEFAULT_MIN_RUNS
  const limit = options?.limit ?? DEFAULT_SUGGESTION_LIMIT
  const groups = new Map<
    string,
    { pattern: string[]; runCount: number; lastUsedAt: number }
  >()

  for (const record of records) {
    const pattern = normalizeToolSequence(record.toolNames)
    if (pattern.length < MIN_PATTERN_LENGTH) continue

    const key = pattern.join('\u001f')
    const existing = groups.get(key)
    groups.set(key, {
      pattern,
      runCount: (existing?.runCount ?? 0) + 1,
      lastUsedAt: Math.max(existing?.lastUsedAt ?? 0, record.recordedAt),
    })
  }

  const suggestions = Array.from(groups.values())
    .filter((group) => group.runCount >= minRuns)
    .map((group, index): WorkflowSkillSuggestion => {
      const pattern = group.pattern
      return {
        id: `workflow-${index + 1}`,
        title: buildSuggestionTitle(pattern),
        runCount: group.runCount,
        pattern,
        lastUsedAt: group.lastUsedAt,
        benefit: buildBenefit(pattern),
      }
    })
    .sort(compareSuggestions)
    .slice(0, limit)

  return {
    totalRuns: records.length,
    eligibleRuns: Array.from(groups.values()).reduce(
      (count, group) => count + group.runCount,
      0,
    ),
    suggestions,
  }
}

export function formatWorkflowAnalysisResponse(
  analysis: WorkflowUsageAnalysis,
): string {
  if (analysis.suggestions.length === 0) {
    return [
      "I don't have enough repeated local tool patterns to suggest a custom skill yet.",
      '',
      PRIVACY_NOTE,
      '',
      `Tracked runs: ${analysis.totalRuns}. Eligible repeated-tool runs: ${analysis.eligibleRuns}.`,
    ].join('\n')
  }

  const suggestionLines = analysis.suggestions.map((suggestion, index) =>
    [
      `${index + 1}. ${suggestion.title} -> ${suggestion.runCount} times`,
      `   Pattern: ${suggestion.pattern.map(labelTool).join(' -> ')}`,
      `   Suggestion: Create a "${suggestion.title}" skill for this exact command sequence.`,
      `   Benefit: ${suggestion.benefit}`,
    ].join('\n'),
  )

  return [
    `Found ${analysis.suggestions.length} potential automation${analysis.suggestions.length === 1 ? '' : 's'}:`,
    '',
    ...suggestionLines,
    '',
    PRIVACY_NOTE,
  ].join('\n')
}

export function formatWorkflowUsageDataResponse(
  records: WorkflowUsageRecord[],
): string {
  if (records.length === 0) {
    return [
      'No local workflow usage patterns are stored yet.',
      '',
      PRIVACY_NOTE,
    ].join('\n')
  }

  const recentRecords = records
    .slice()
    .sort((left, right) => right.recordedAt - left.recordedAt)
    .slice(0, 10)

  return [
    `Stored local workflow runs: ${records.length}`,
    '',
    ...recentRecords.map(
      (record, index) =>
        `${index + 1}. ${normalizeToolSequence(record.toolNames).map(labelTool).join(' -> ')}`,
    ),
    '',
    PRIVACY_NOTE,
  ].join('\n')
}

export function formatWorkflowUsageClearedResponse(): string {
  return 'Cleared the local workflow usage pattern data. No URLs, page content, prompts, tool inputs, or tool outputs were stored in this data.'
}
