import { Check, Code, Copy, Sparkles } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { AgentRow } from '@/modules/api/agents.hooks'

interface CopyFromExistingCardProps {
  agents: readonly AgentRow[]
  selectedId: string | null
  onClone: (agent: AgentRow) => void
}

export function CopyFromExistingCard({
  agents,
  selectedId,
  onClone,
}: CopyFromExistingCardProps) {
  if (agents.length === 0) return null
  return (
    <div className="rounded-2xl border border-accent-tint-2 bg-gradient-to-br from-accent-tint to-[hsl(35_90%_96%)] p-4">
      <div className="mb-1 flex items-center gap-2">
        <Copy className="size-4 text-accent" />
        <span className="font-semibold text-ink text-sm">
          Copy from an existing agent
        </span>
      </div>
      <p className="mb-3 text-ink-2 text-xs leading-snug">
        Clone the logins, guardrails and ACL rules of an agent you already
        trust, then tweak.
      </p>
      <div className="flex flex-wrap gap-2">
        {agents.map((agent) => {
          const selected = selectedId === agent.id
          const isCodex = agent.harness === 'Codex'
          const HarnessIcon = selected ? Check : isCodex ? Code : Sparkles
          return (
            <button
              key={agent.id}
              type="button"
              onClick={() => onClone(agent)}
              className={cn(
                'flex max-w-[220px] items-center gap-2 rounded-lg border p-2 text-left transition-colors',
                selected
                  ? 'border-accent bg-card'
                  : 'border-border-2 bg-card/60 hover:border-accent/60 hover:bg-card',
              )}
            >
              <HarnessIcon
                className={cn(
                  'size-3.5 shrink-0',
                  selected
                    ? 'text-green'
                    : isCodex
                      ? 'text-ink-3'
                      : 'text-accent',
                )}
              />
              <span className="min-w-0 flex-1">
                <span className="block truncate font-semibold text-xs">
                  {agent.label}
                </span>
                <span className="text-[10.5px] text-ink-3">
                  {agent.harness}
                </span>
              </span>
            </button>
          )
        })}
      </div>
    </div>
  )
}
