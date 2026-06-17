import { Check, Copy } from 'lucide-react'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import { cn } from '@/lib/utils'
import type { AgentProfile } from '@/modules/api/agents.hooks'

interface CopyFromExistingCardProps {
  profiles: readonly AgentProfile[]
  selectedId: string | null
  onClone: (profile: AgentProfile) => void
}

export function CopyFromExistingCard({
  profiles,
  selectedId,
  onClone,
}: CopyFromExistingCardProps) {
  if (profiles.length === 0) return null
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
        {profiles.map((profile) => {
          const selected = selectedId === profile.id
          return (
            <button
              key={profile.id}
              type="button"
              onClick={() => onClone(profile)}
              className={cn(
                'flex max-w-[220px] items-center gap-2 rounded-lg border p-2 text-left transition-colors',
                selected
                  ? 'border-accent bg-card'
                  : 'border-border-2 bg-card/60 hover:border-accent/60 hover:bg-card',
              )}
            >
              {selected ? (
                <Check className="size-3.5 shrink-0 text-green" />
              ) : (
                <HarnessIcon
                  harness={profile.harness}
                  className="size-3.5 shrink-0 text-accent"
                />
              )}
              <span className="min-w-0 flex-1">
                <span className="block truncate font-semibold text-xs">
                  {profile.name}
                </span>
                <span className="text-[10.5px] text-ink-3">
                  {profile.harness}
                </span>
              </span>
            </button>
          )
        })}
      </div>
    </div>
  )
}
