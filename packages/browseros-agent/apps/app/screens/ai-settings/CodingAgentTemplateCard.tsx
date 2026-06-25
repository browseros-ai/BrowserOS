import type { FC } from 'react'
import { AdapterIcon, adapterLabel } from '@/components/agents/AdapterIcon'
import { Badge } from '@/components/ui/badge'
import type {
  HarnessAdapterDescriptor,
  HarnessAgentAdapter,
} from '@/modules/agents/agent-harness-types'

export interface CodingAgentTemplateCardProps {
  adapter: HarnessAdapterDescriptor
  onCreate: (adapterId: HarnessAgentAdapter) => void
}

/**
 * A coding-agent (Claude Code / Codex) entry rendered inside the provider
 * templates grid. Visually identical to ProviderTemplateCard, but clicking it
 * opens the New Agent dialog instead of configuring an LLM provider.
 */
export const CodingAgentTemplateCard: FC<CodingAgentTemplateCardProps> = ({
  adapter,
  onCreate,
}) => {
  return (
    <button
      type="button"
      onClick={() => onCreate(adapter.id)}
      className="group relative flex w-full items-center gap-3 rounded-lg border border-border bg-background p-4 text-left transition-all hover:border-[var(--accent-orange)] hover:shadow-md"
    >
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <AdapterIcon
          adapter={adapter.id}
          className="size-7 shrink-0 text-accent-orange/70 transition-colors group-hover:text-accent-orange"
        />
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium text-foreground">
              {adapter.name || adapterLabel(adapter.id)}
            </span>
          </div>
        </div>
      </div>
      <Badge
        variant="outline"
        className="shrink-0 rounded-md px-3 py-1 transition-colors group-hover:border-[var(--accent-orange)] group-hover:text-[var(--accent-orange)]"
      >
        USE
      </Badge>
    </button>
  )
}
