import { Bot, Cpu, Sparkles } from 'lucide-react'
import type { FC } from 'react'
import type { HarnessAgentAdapter } from '@/modules/agents/agent-harness-types'

/**
 * Single icon component for any adapter the agent rail can render.
 * Falls back to a generic bot when the adapter is unknown so future
 * adapters land without a code change at the call site.
 */
export interface AdapterIconProps {
  adapter: HarnessAgentAdapter | 'unknown'
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

export function adapterLabel(adapter: HarnessAgentAdapter | 'unknown'): string {
  switch (adapter) {
    case 'claude':
      return 'Claude Code'
    case 'codex':
      return 'Codex'
    default:
      return 'Agent'
  }
}
