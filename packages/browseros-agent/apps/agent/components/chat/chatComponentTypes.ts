import type { ProviderType } from '@/lib/llm-providers/types'

export type ChatProviderType = ProviderType | 'acp'

export interface Provider {
  id: string
  name: string
  type: ChatProviderType
  kind: 'llm' | 'acp'
  modelControl?: 'runtime-supported' | 'best-effort'
}
