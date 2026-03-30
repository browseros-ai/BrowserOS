import type { CaptureContext } from '../capture/context'
import type { EvalConfig, Message, Task, TaskMetadata } from '../types'

/**
 * All dependencies an agent evaluator needs - passed via factory
 */
export interface AgentContext {
  // Configuration
  config: EvalConfig
  task: Task

  // Page resolved once at task start (fresh browser has exactly one page)
  initialPageId: number

  // Browser window info for agents that operate on explicit window/tab ids
  windowId?: number
  tabId?: number

  // Output paths
  outputDir: string // Root output directory
  taskOutputDir: string // Task-specific: outputDir/query_id/

  // Capture infrastructure (pre-initialized by runner)
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
