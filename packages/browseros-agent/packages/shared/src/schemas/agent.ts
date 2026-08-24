import { z } from 'zod'

export const AcpAgentTypeSchema: z.ZodEnum<['claude', 'codex', 'custom']> =
  z.enum(['claude', 'codex', 'custom'])

/**
 * Config for a user-defined ("custom") ACP agent. Only meaningful when the
 * agent's type is 'custom'; the built-in claude/codex agents leave this null.
 * `command` is the full launch command line (args included); it is shell-split
 * into argv at launch and probe time.
 */
export const CustomAcpAgentConfigSchema: z.ZodObject<
  {
    command: z.ZodString
    env: z.ZodOptional<z.ZodRecord<z.ZodString, z.ZodString>>
    fullAccessModes: z.ZodOptional<z.ZodArray<z.ZodString>>
    reasoningEffortKey: z.ZodOptional<z.ZodString>
    systemPromptAppend: z.ZodOptional<z.ZodString>
    icon: z.ZodOptional<z.ZodString>
  },
  'strict'
> = z
  .object({
    command: z.string().trim().min(1),
    env: z.record(z.string(), z.string()).optional(),
    fullAccessModes: z.array(z.string().trim().min(1)).optional(),
    reasoningEffortKey: z.string().trim().min(1).optional(),
    systemPromptAppend: z.string().trim().min(1).optional(),
    icon: z.string().trim().min(1).max(64).optional(),
  })
  .strict()

export const BrowserOsAgentTargetSchema: z.ZodObject<{
  type: z.ZodLiteral<'browseros'>
  providerId: z.ZodString
}> = z.object({
  type: z.literal('browseros'),
  providerId: z.string().min(1),
})

export const ClaudeAgentTargetSchema: z.ZodObject<{
  type: z.ZodLiteral<'claude'>
  agentId: z.ZodString
}> = z.object({
  type: z.literal('claude'),
  agentId: z.string().uuid(),
})

export const CodexAgentTargetSchema: z.ZodObject<{
  type: z.ZodLiteral<'codex'>
  agentId: z.ZodString
}> = z.object({
  type: z.literal('codex'),
  agentId: z.string().uuid(),
})

export const CustomAgentTargetSchema: z.ZodObject<{
  type: z.ZodLiteral<'custom'>
  agentId: z.ZodString
}> = z.object({
  type: z.literal('custom'),
  agentId: z.string().uuid(),
})

export const AgentTargetSchema: z.ZodDiscriminatedUnion<
  'type',
  [
    typeof BrowserOsAgentTargetSchema,
    typeof ClaudeAgentTargetSchema,
    typeof CodexAgentTargetSchema,
    typeof CustomAgentTargetSchema,
  ]
> = z.discriminatedUnion('type', [
  BrowserOsAgentTargetSchema,
  ClaudeAgentTargetSchema,
  CodexAgentTargetSchema,
  CustomAgentTargetSchema,
])

export const AcpAgentTargetSchema: z.ZodDiscriminatedUnion<
  'type',
  [
    typeof ClaudeAgentTargetSchema,
    typeof CodexAgentTargetSchema,
    typeof CustomAgentTargetSchema,
  ]
> = z.discriminatedUnion('type', [
  ClaudeAgentTargetSchema,
  CodexAgentTargetSchema,
  CustomAgentTargetSchema,
])

export type AcpAgentType = z.infer<typeof AcpAgentTypeSchema>
export type CustomAcpAgentConfig = z.infer<typeof CustomAcpAgentConfigSchema>
export type AgentTarget = z.infer<typeof AgentTargetSchema>
export type AcpAgentTarget = z.infer<typeof AcpAgentTargetSchema>
