/**
 * Types for Orchestrator-Executor pattern
 */

export interface ExecutorResult {
  observation: string
  status: 'done' | 'blocked' | 'timeout'
  url: string
  actionsPerformed: number
  toolsUsed: string[]
}

export interface ExecutorConfig {
  provider: string
  model: string
  apiKey: string
  baseUrl?: string
}

export const ORCHESTRATOR_DEFAULTS = {
  maxTurns: 15,
} as const

export const LIMITS = {
  maxTotalSteps: 300,
  delegationTimeoutMs: 300_000,
} as const

/**
 * Factory function type for creating executor runs.
 * Built in index.ts with Browser + capture callbacks captured in closure.
 */
export type ExecutorFactory = (
  instruction: string,
  signal: AbortSignal,
) => Promise<ExecutorResult>
