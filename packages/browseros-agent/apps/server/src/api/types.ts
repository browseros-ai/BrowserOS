/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  type BrowserContext,
  BrowserContextSchema,
  type CustomMcpServer,
  CustomMcpServerSchema,
  type Tab,
  TabSchema,
} from '@browseros/shared/schemas/browser-context'
import { LLMConfigSchema } from '@browseros/shared/schemas/llm'
import { z } from 'zod'
import type { ControllerBackend } from '../browser/backends/controller'
import type { Browser } from '../browser/browser'
import type { RateLimiter } from '../lib/rate-limiter/rate-limiter'
import type { ToolRegistry } from '../tools/tool-registry'

// Re-export browser context types for consumers
export {
  type BrowserContext,
  BrowserContextSchema,
  type CustomMcpServer,
  CustomMcpServerSchema,
  type Tab,
  TabSchema,
}

export const AgentLLMConfigSchema = LLMConfigSchema.extend({
  model: z.string().min(1, 'Model name is required'),
  upstreamProvider: z.string().optional(),
})

export type AgentLLMConfig = z.infer<typeof AgentLLMConfigSchema>

export const ChatRequestSchema = AgentLLMConfigSchema.extend({
  conversationId: z.string().uuid(),
  message: z.string().min(1, 'Message cannot be empty'),
  contextWindowSize: z.number().optional(),
  browserContext: BrowserContextSchema.optional(),
  userSystemPrompt: z.string().optional(),
  isScheduledTask: z.boolean().optional().default(false),
  userWorkingDir: z.string().min(1).optional(),
  supportsImages: z.boolean().optional().default(true),
  mode: z.enum(['chat', 'agent']).optional().default('agent'),
  declinedApps: z.array(z.string()).optional(),
  previousConversation: z
    .union([
      z.array(
        z.object({
          role: z.enum(['user', 'assistant']),
          content: z.string(),
        }),
      ),
      z.string(),
    ])
    .optional()
    .transform((val) => {
      if (typeof val !== 'string') return val
      if (!val.trim()) return undefined
      return [{ role: 'user' as const, content: val }]
    }),
})

export type ChatRequest = z.infer<typeof ChatRequestSchema>

/**
 * Hono environment bindings for Bun.serve integration.
 */
export type Env = {
  Bindings: {
    server: ReturnType<typeof Bun.serve>
  }
}

/**
 * Configuration for the consolidated HTTP server.
 * This server handles all routes: health, klavis, chat, mcp, provider
 */
export interface HttpServerConfig {
  port: number
  host?: string

  version: string
  browser: Browser
  controller: ControllerBackend
  registry: ToolRegistry

  browserosId?: string
  executionDir: string
  resourcesDir: string
  rateLimiter?: RateLimiter

  codegenServiceUrl?: string
  aiSdkDevtoolsEnabled?: boolean

  onShutdown?: () => void
}

// Graph request schemas
export const CreateGraphRequestSchema = z.object({
  query: z.string().min(1, 'Query cannot be empty'),
})

export type CreateGraphRequest = z.infer<typeof CreateGraphRequestSchema>

export const UpdateGraphRequestSchema = z.object({
  query: z.string().min(1, 'Query cannot be empty'),
})

export type UpdateGraphRequest = z.infer<typeof UpdateGraphRequestSchema>

// Run graph request - similar to ChatRequest, needs provider config for Agent SDK
export const RunGraphRequestSchema = AgentLLMConfigSchema.extend({
  browserContext: BrowserContextSchema.optional(),
})

export type RunGraphRequest = z.infer<typeof RunGraphRequestSchema>

// Workflow graph schemas (matching codegen-service)
export const WorkflowNodeTypeSchema = z.enum([
  'start',
  'end',
  'nav',
  'act',
  'extract',
  'verify',
  'decision',
  'loop',
  'fork',
  'join',
])

export type WorkflowNodeType = z.infer<typeof WorkflowNodeTypeSchema>

export const WorkflowNodeSchema = z.object({
  id: z.string(),
  type: WorkflowNodeTypeSchema,
  data: z.object({ label: z.string() }),
})

export type WorkflowNode = z.infer<typeof WorkflowNodeSchema>

export const WorkflowEdgeSchema = z.object({
  id: z.string(),
  source: z.string(),
  target: z.string(),
})

export type WorkflowEdge = z.infer<typeof WorkflowEdgeSchema>

export const WorkflowGraphSchema = z.object({
  nodes: z.array(WorkflowNodeSchema),
  edges: z.array(WorkflowEdgeSchema),
})

export type WorkflowGraph = z.infer<typeof WorkflowGraphSchema>

export interface GraphSession {
  id: string
  code: string
  graph: WorkflowGraph | null
  createdAt: Date
}

// Codegen service response schema for GET /api/code/:id
export const CodegenGetResponseSchema = z.object({
  code: z.string(),
  graph: WorkflowGraphSchema.nullable(),
  createdAt: z.string().optional(),
})

export type CodegenGetResponse = z.infer<typeof CodegenGetResponseSchema>

// Metadata schema for finish events from codegen service
export const CodegenFinishMetadataSchema = z.object({
  codeId: z.string().optional(),
  code: z.string().optional(),
  graph: WorkflowGraphSchema.nullable().optional(),
})

export type CodegenFinishMetadata = z.infer<typeof CodegenFinishMetadataSchema>
