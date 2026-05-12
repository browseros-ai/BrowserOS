import type { AgentResult } from '../agents/types'
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

export const PASS_FAIL_GRADER_ORDER = [
  'agisdk_state_diff',
  'infinity_state',
  'performance_grader',
  'webvoyager_grader',
  'fara_combined',
  'fara_grader',
] as const

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

// ============================================================================
// Grader Options
// ============================================================================

export interface GraderOptions {
  apiKey: string
  baseUrl?: string
  model?: string
}

export function resolveGraderOptions(config: EvalConfig): GraderOptions | null {
  const keyValue = config.grader_api_key_env || 'OPENAI_API_KEY'
  // If it looks like an env var name (ALL_CAPS), resolve from env; otherwise use directly
  const apiKey = /^[A-Z][A-Z0-9_]*$/.test(keyValue)
    ? process.env[keyValue]
    : keyValue

  if (!apiKey) {
    return null
  }

  return {
    apiKey,
    baseUrl: config.grader_base_url,
    model: config.grader_model,
  }
}
