import { z } from 'zod'

export const AcpAgentTypeSchema: z.ZodEnum<['claude', 'codex']> = z.enum([
  'claude',
  'codex',
])

const BrowserOsAgentTargetSchema: z.ZodObject<{
  type: z.ZodLiteral<'browseros'>
  providerId: z.ZodString
}> = z.object({
  type: z.literal('browseros'),
  providerId: z.string().min(1),
})

const ClaudeAgentTargetSchema: z.ZodObject<{
  type: z.ZodLiteral<'claude'>
  agentId: z.ZodString
}> = z.object({
  type: z.literal('claude'),
  agentId: z.string().uuid(),
})

const CodexAgentTargetSchema: z.ZodObject<{
  type: z.ZodLiteral<'codex'>
  agentId: z.ZodString
}> = z.object({
  type: z.literal('codex'),
  agentId: z.string().uuid(),
})

export const AgentTargetSchema: z.ZodDiscriminatedUnion<
  'type',
  [
    typeof BrowserOsAgentTargetSchema,
    typeof ClaudeAgentTargetSchema,
    typeof CodexAgentTargetSchema,
  ]
> = z.discriminatedUnion('type', [
  BrowserOsAgentTargetSchema,
  ClaudeAgentTargetSchema,
  CodexAgentTargetSchema,
])

export type AcpAgentType = z.infer<typeof AcpAgentTypeSchema>
export type AgentTarget = z.infer<typeof AgentTargetSchema>
