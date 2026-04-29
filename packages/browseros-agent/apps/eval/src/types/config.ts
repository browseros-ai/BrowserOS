import {
  LLMConfigSchema,
  LLMProviderSchema,
} from '@browseros/shared/schemas/llm'
import { z } from 'zod'

export const SingleAgentConfigSchema = LLMConfigSchema.extend({
  type: z.literal('single'),
  supportsImages: z.boolean().optional(),
})

export const OrchestratorExecutorConfigSchema = z.object({
  type: z.literal('orchestrator-executor'),
  orchestrator: LLMConfigSchema.extend({
    maxTurns: z.number().int().min(1).optional(),
  }),
  executor: LLMConfigSchema.extend({
    provider: z.union([LLMProviderSchema, z.literal('clado-action')]),
  }),
})

export const AgentConfigSchema = z.discriminatedUnion('type', [
  SingleAgentConfigSchema,
  OrchestratorExecutorConfigSchema,
])

export const EvalConfigSchema = z.object({
  agent: AgentConfigSchema,
  dataset: z.string().min(1),
  output_dir: z.string().optional(),
  num_workers: z.number().int().min(1).max(20).default(1),
  restart_server_per_task: z.boolean().optional().default(false),
  browseros: z.object({
    server_url: z.string().url(),
    base_cdp_port: z.number().int().optional().default(9010),
    base_server_port: z.number().int().optional().default(9110),
    base_extension_port: z.number().int().optional().default(9310),
    load_extensions: z.boolean().optional().default(false),
    headless: z.boolean().optional().default(false),
  }),
  graders: z.array(z.string()).optional(),
  timeout_ms: z.number().int().min(30000).max(3600000).optional(),
  captcha: z
    .object({
      api_key_env: z.string().default('NOPECHA_API_KEY'),
      wait_timeout_ms: z.number().int().min(1000).max(120000).default(30000),
      poll_interval_ms: z.number().int().min(200).max(5000).default(1000),
    })
    .optional(),
})

export type SingleAgentConfig = z.infer<typeof SingleAgentConfigSchema>
export type OrchestratorExecutorConfig = z.infer<
  typeof OrchestratorExecutorConfigSchema
>
export type AgentConfig = z.infer<typeof AgentConfigSchema>
export type EvalConfig = z.infer<typeof EvalConfigSchema>
