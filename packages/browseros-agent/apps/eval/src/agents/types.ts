import type { CaptureContext } from '../capture/context'
import type { EvalConfig, Message, Task, TaskMetadata } from '../types'

export interface AgentContext {
  config: EvalConfig
  task: Task
  workerIndex: number

  // Resolved once at task start (fresh browser has exactly one page).
  initialPageId: number

  outputDir: string // Root output directory
  taskOutputDir: string // Task-specific: outputDir/query_id/

  capture: CaptureContext
}

/**
 * Result returned by agent execution
 */
export interface AgentResult {
  metadata: TaskMetadata
  messages: Message[]
  finalAnswer: string | null
}

/**
 * Interface all agent evaluators must implement
 */
export interface AgentEvaluator {
  execute(): Promise<AgentResult>
}
