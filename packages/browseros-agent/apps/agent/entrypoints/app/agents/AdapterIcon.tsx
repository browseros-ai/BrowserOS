import { Bot, Cpu, Plug, Sparkles, Wand2 } from 'lucide-react'
import type { FC } from 'react'
import type { HarnessAgentAdapter } from './agent-harness-types'

/**
 * Single icon component for any adapter the agent rail can render.
 * Falls back to a generic bot when the adapter is unknown so future
 * adapters land without a code change at the call site.
 */
interface AdapterIconProps {
  adapter: HarnessAgentAdapter | 'unknown'
  className?: string
}

export const AdapterIcon: FC<AdapterIconProps> = ({ adapter, className }) => {
  switch (adapter) {
    case 'claude':
      // Claude Code — text-based agent, sparkles to evoke the "AI assistant" feel.
      return <Sparkles className={className} aria-label="Claude Code" />
    case 'codex':
      // Codex — code-leaning, CPU mark.
      return <Cpu className={className} aria-label="Codex" />
    case 'openclaw':
      // OpenClaw — bot/automation framing.
      return <Bot className={className} aria-label="OpenClaw" />
    case 'hermes':
      // Hermes — messenger god framing, wand evokes the agentic conjuring.
      return <Wand2 className={className} aria-label="Hermes" />
    case 'custom':
      // Custom ACP — plug/terminal icon for user-provided binary.
      return <Plug className={className} aria-label="Custom ACP" />
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
    case 'openclaw':
      return 'OpenClaw'
    case 'hermes':
      return 'Hermes'
    case 'custom':
      return 'Custom ACP'
    default:
      return 'Agent'
  }
}
