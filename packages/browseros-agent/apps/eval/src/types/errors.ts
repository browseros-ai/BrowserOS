import { z } from 'zod'

// Error source - where the error originated
const ErrorSourceSchema = z.enum([
  'window_creation',
  'navigation',
  'agent_execution',
  'mcp_tool',
  'screenshot',
  'grader',
  'message_logging',
  'cleanup',
  'unknown',
])

export type ErrorSource = z.infer<typeof ErrorSourceSchema>

// Task error with details
export const TaskErrorSchema = z.object({
  source: ErrorSourceSchema,
  message: z.string(),
  timestamp: z.string(),
  details: z.record(z.unknown()).optional(),
})

export type TaskError = z.infer<typeof TaskErrorSchema>

// Evaluation warning (non-fatal)
export const EvalWarningSchema = z.object({
  source: ErrorSourceSchema,
  message: z.string(),
  timestamp: z.string(),
})

export type EvalWarning = z.infer<typeof EvalWarningSchema>
