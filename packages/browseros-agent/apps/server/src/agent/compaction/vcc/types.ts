export interface FileOps {
  readFiles?: string[]
  modifiedFiles?: string[]
  createdFiles?: string[]
}

export type NormalizedBlock =
  | { kind: 'user'; text: string; sourceIndex?: number }
  | { kind: 'assistant'; text: string; sourceIndex?: number }
  | {
      kind: 'tool_call'
      name: string
      args: Record<string, unknown>
      sourceIndex?: number
    }
  | {
      kind: 'tool_result'
      name: string
      text: string
      isError: boolean
      sourceIndex?: number
    }
  | {
      kind: 'thinking'
      text: string
      redacted: boolean
      sourceIndex?: number
    }

export interface TranscriptEntry {
  role: 'user' | 'assistant' | 'tool_error'
  text?: string
  tool?: string
  cmd?: string
  ref?: string
  count?: number
}

export interface SiteVisit {
  url: string
  domain: string
  order: number
  sourceIndex: number
  tools: Map<string, number>
}

export interface SectionData {
  sessionGoal: string[]
  outstandingContext: string[]
  filesAndChanges: string[]
  commits: string[]
  siteActivity: SiteVisit[]
  userPreferences: string[]
  briefTranscript: string
  transcriptEntries: TranscriptEntry[]
}

export interface VccOverrides {
  maxTranscriptLines?: number
  maxGoalLines?: number
  maxFileEntries?: number
  maxCommitEntries?: number
  maxPreferenceLines?: number
  maxOutstandingLines?: number
}
