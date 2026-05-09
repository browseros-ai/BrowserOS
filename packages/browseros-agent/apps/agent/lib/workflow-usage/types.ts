export type WorkflowUsageSource = 'sidepanel-chat' | 'agent-harness-chat'

export interface WorkflowUsageRecord {
  id: string
  source: WorkflowUsageSource
  recordedAt: number
  toolNames: string[]
}

export interface WorkflowUsageStore {
  version: 1
  records: WorkflowUsageRecord[]
}

export interface WorkflowSkillSuggestion {
  id: string
  title: string
  runCount: number
  pattern: string[]
  lastUsedAt: number
  benefit: string
}

export interface WorkflowUsageAnalysis {
  totalRuns: number
  eligibleRuns: number
  suggestions: WorkflowSkillSuggestion[]
}
