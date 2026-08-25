import type { ProviderType } from '@/lib/llm-providers/types'

export type ChatProviderType = ProviderType | 'acp'

export interface Provider {
  id: string
  name: string
  type: ChatProviderType
  kind: 'llm' | 'acp'
  agentId?: string
  adapterName?: string
  /** Brand id for the agent's logo (its type, or a popular-agent id). */
  brandKey?: string
  modelLabel?: string
  modelControl?: 'runtime-supported' | 'best-effort'
}
