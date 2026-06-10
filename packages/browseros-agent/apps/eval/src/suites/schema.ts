import { z } from 'zod'
import { EvalConfigSchema } from '../types'

const SuiteAgentSchema = z
  .object({
    type: z.enum([
      'tool-loop',
      'single',
      'orchestrated',
      'orchestrator-executor',
      'claude-code',
    ]),
    executorBackend: z.enum(['tool-loop', 'clado']).optional(),
  })
  .superRefine((agent, ctx) => {
    if (
      (agent.type === 'orchestrated' ||
        agent.type === 'orchestrator-executor') &&
      !agent.executorBackend
    ) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ['executorBackend'],
        message: 'executorBackend is required for orchestrated suites',
      })
    }
  })

export const EvalSuiteSchema = z.object({
  id: z.string().min(1),
  dataset: z.string().min(1),
  agent: SuiteAgentSchema,
  graders: z.array(z.string()).default([]),
  workers: z.number().int().min(1).max(20).default(1),
  restartBrowserPerTask: z.boolean().default(false),
  timeoutMs: z.number().int().min(30_000).max(3_600_000).optional(),
  browseros: EvalConfigSchema.shape.browseros.optional(),
  captcha: EvalConfigSchema.shape.captcha.optional(),
})

export type EvalSuite = z.infer<typeof EvalSuiteSchema>
