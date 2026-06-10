import { z } from 'zod'

// Task metadata schema
const TaskInputMetadataSchema = z.object({
  original_task_id: z.string(),
  website: z.string().optional(),
  category: z.string().optional(),
  additional: z.record(z.unknown()).optional(),
})

// Task schema (from dataset JSONL)
export const TaskSchema = z.object({
  query_id: z.string(),
  dataset: z.string(),
  query: z.string(),
  graders: z.array(z.string()).optional().default([]),
  start_url: z.string().optional(),
  setup_script: z.string().optional(),
  metadata: TaskInputMetadataSchema,
})

export type TaskInputMetadata = z.infer<typeof TaskInputMetadataSchema>
export type Task = z.infer<typeof TaskSchema>
