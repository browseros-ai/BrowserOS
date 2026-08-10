import { Bot, Cpu, Sparkles } from 'lucide-react'
import type { FC } from 'react'
import type { AcpAgentType } from '@/modules/agents/acp-agent-types'

export interface AdapterIconProps {
  adapter: AcpAgentType | 'unknown'
  className?: string
}

export const AdapterIcon: FC<AdapterIconProps> = ({ adapter, className }) => {
  switch (adapter) {
    case 'claude':
      return <Sparkles className={className} aria-label="Claude Code" />
    case 'codex':
      return <Cpu className={className} aria-label="Codex" />
    default:
      return <Bot className={className} aria-label="Agent" />
  }
}

export function adapterLabel(adapter: AcpAgentType | 'unknown'): string {
  switch (adapter) {
    case 'claude':
      return 'Claude Code'
    case 'codex':
      return 'Codex'
    default:
      return 'Agent'
  }
}
