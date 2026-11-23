import { z } from 'zod'

const AcpAgentTypeSchema = z.enum(['claude', 'codex', 'custom'])

export type AcpAgentType = z.infer<typeof AcpAgentTypeSchema>
