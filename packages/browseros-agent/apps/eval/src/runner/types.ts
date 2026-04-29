import type { AgentResult } from '../agents/types'
import { PASS_FAIL_GRADER_ORDER } from '../graders/registry'
import type { ErrorSource, EvalConfig, GraderResult, Task } from '../types'

// ============================================================================
// Runner Options
// ============================================================================

export interface RunEvalOptions {
  configPath: string
  dataPath?: string
  query?: string
  startUrl?: string
  outputDir?: string
}

// ============================================================================
// Task Loading
// ============================================================================

export type TaskSource =
  | { type: 'file'; path: string }
  | { type: 'single'; query: string; startUrl?: string }

export interface TaskLoadResult {
  tasks: Task[]
  source: TaskSource
}

// ============================================================================
// Task Execution
// ============================================================================

export interface TaskExecutionContext {
  task: Task
  config: EvalConfig
  outputDir: string
}

export type TaskResult =
  | {
      status: 'completed'
      task: Task
      agentResult: AgentResult
      graderResults: Record<string, GraderResult>
      durationMs: number
    }
  | {
      status: 'timeout'
      task: Task
      agentResult: AgentResult
      graderResults: Record<string, GraderResult>
      durationMs: number
    }
  | {
      status: 'failed'
      task: Task
      error: Error
      errorSource: ErrorSource
      durationMs: number
    }

// Type guard for successful results
export function isSuccessfulResult(
  result: TaskResult,
): result is TaskResult & { status: 'completed' | 'timeout' } {
  return result.status === 'completed' || result.status === 'timeout'
}

// ============================================================================
// Batch Summary
// ============================================================================

export interface BatchSummary {
  total: number
  completed: number
  failed: number
  timeout: number
  passRate: number
  avgDurationMs: number
  // Error breakdown by source
  errorsBySource: Partial<Record<ErrorSource, number>>
  totalWarnings: number
  results: TaskResultSummary[]
}

export interface TaskResultSummary {
  queryId: string
  status: TaskResult['status']
  durationMs: number
  graderResults?: Record<string, { pass: boolean; score: number }>
  // Error tracking
  errorCount: number
  warningCount: number
  errorSources?: ErrorSource[]
  failureReason?: string
}

// ============================================================================
// Pass/Fail Determination
// ============================================================================

export function getPrimaryGraderResult(
  graderResults: Record<string, { pass: boolean; score: number }>,
): { name: string; pass: boolean; score: number } | null {
  for (const name of PASS_FAIL_GRADER_ORDER) {
    if (graderResults[name]) {
      return { name, ...graderResults[name] }
    }
  }
  const first = Object.entries(graderResults)[0]
  if (first) {
    return { name: first[0], ...first[1] }
  }
  return null
}
