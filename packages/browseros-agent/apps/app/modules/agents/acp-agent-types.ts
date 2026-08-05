import type { AcpAgentType } from '@browseros/shared/schemas/agent'

export type { AcpAgentType }

export interface AcpAgent {
  id: string
  name: string
  type: AcpAgentType
  modelId?: string
  reasoningEffort?: string
  workingDirectory?: string
  createdAt: number
  updatedAt: number
}

export interface AcpProbeResult {
  models: Array<{ id: string; name?: string; description?: string }>
  reasoning: { values: string[]; defaultValue?: string } | null
  supportsConfigOption: boolean
  agentInfo: { name?: string; title?: string; version?: string } | null
  protocolVersion: number
  error?: { code: string; message: string }
}
