import { z } from 'zod'
import { EvalWarningSchema, TaskErrorSchema } from './errors'

// Grader result
const GraderResultSchema = z.object({
  score: z.number(),
  pass: z.boolean(),
  reasoning: z.string(),
  details: z.record(z.unknown()).optional(),
})

// Agent config in metadata
const AgentConfigMetaSchema = z
  .object({
    type: z.enum(['single', 'orchestrator-executor', 'claude-code']),
    model: z.string().optional(),
  })
  .passthrough()

// LLM token consumption for the task (summed across all LLM calls)
const TokenUsageSchema = z.object({
  input_tokens: z.number(),
  output_tokens: z.number(),
  cache_read_tokens: z.number(),
  cache_creation_tokens: z.number(),
})

// Dataset-derived metadata passed through to the task (AGI SDK fields, etc.)
const TaskDatasetMetadataSchema = z
  .object({
    website: z.string().optional(),
    difficulty: z.string().optional(),
    challenge_type: z.string().optional(),
    similar_to: z.string().optional(),
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
  token_usage: TokenUsageSchema.optional(),
  task_metadata: TaskDatasetMetadataSchema.optional(),
})

// Export types
export type GraderResult = z.infer<typeof GraderResultSchema>
export type TaskMetadata = z.infer<typeof TaskMetadataSchema>
export type TokenUsage = z.infer<typeof TokenUsageSchema>
export type TaskDatasetMetadata = z.infer<typeof TaskDatasetMetadataSchema>
