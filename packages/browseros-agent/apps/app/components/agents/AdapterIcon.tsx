import { Blocks, Bot } from 'lucide-react'
import type { FC } from 'react'
import type { AcpAgentType } from '@/modules/agents/acp-agent-types'
import { BRAND_MARKS } from './agent-brand-marks'

export interface AdapterIconProps {
  adapter: AcpAgentType | 'unknown'
  className?: string
}

export const AdapterIcon: FC<AdapterIconProps> = ({ adapter, className }) => {
  const Mark = BRAND_MARKS[adapter]
  if (Mark) return <Mark className={className} />
  if (adapter === 'custom') {
    return <Blocks className={className} aria-label="Custom agent" />
  }
  return <Bot className={className} aria-label="Agent" />
}

export function adapterLabel(adapter: AcpAgentType | 'unknown'): string {
  switch (adapter) {
    case 'claude':
      return 'Claude Code'
    case 'codex':
      return 'Codex'
    case 'custom':
      return 'Custom agent'
    default:
      return 'Agent'
  }
}
