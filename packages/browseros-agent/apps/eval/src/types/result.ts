import { z } from 'zod'
import { EvalWarningSchema, TaskErrorSchema } from './errors'
import { MessageSchema } from './message'

// Grader result
export const GraderResultSchema = z.object({
  score: z.number(),
  pass: z.boolean(),
  reasoning: z.string(),
  details: z.record(z.unknown()).optional(),
})

// Agent config in metadata
const AgentConfigMetaSchema = z
  .object({
    type: z.enum(['single', 'orchestrator-executor']),
    model: z.string().optional(),
  })
  .passthrough()

// Task metadata (output)
export const TaskMetadataSchema = z.object({
  query_id: z.string(),
  dataset: z.string(),
  query: z.string(),
  started_at: z.string(),
  completed_at: z.string(),
  total_duration_ms: z.number(),
  total_steps: z.number(),
  screenshot_count: z.number().optional(),
  termination_reason: z.enum(['completed', 'max_steps', 'error', 'timeout']),
  final_answer: z.string().nullable(),
  errors: z.array(TaskErrorSchema),
  warnings: z.array(EvalWarningSchema),
  device_pixel_ratio: z.number().optional(),
  agent_config: AgentConfigMetaSchema,
  grader_results: z.record(GraderResultSchema),
})

// Agent result
export const AgentResultSchema = z.object({
  metadata: TaskMetadataSchema,
  messages: z.array(MessageSchema),
  finalAnswer: z.string().nullable(),
})

// Export types
export type GraderResult = z.infer<typeof GraderResultSchema>
export type TaskMetadata = z.infer<typeof TaskMetadataSchema>
export type AgentResult = z.infer<typeof AgentResultSchema>
