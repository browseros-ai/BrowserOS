import type { FC } from 'react'
import { AdapterIcon, adapterLabel } from '@/components/agents/AdapterIcon'
import { Badge } from '@/components/ui/badge'
import type { AcpAgentType } from '@/modules/agents/acp-agent-types'

export interface CodingAgentTemplateCardProps {
  type: AcpAgentType
  onCreate: (type: AcpAgentType) => void
}

export const CodingAgentTemplateCard: FC<CodingAgentTemplateCardProps> = ({
  type,
  onCreate,
}) => (
  <button
    type="button"
    onClick={() => onCreate(type)}
    className="group relative flex w-full items-center gap-3 rounded-lg border border-border bg-background p-4 text-left transition-all hover:border-[var(--accent-orange)] hover:shadow-md"
  >
    <div className="flex min-w-0 flex-1 items-center gap-3">
      <AdapterIcon
        adapter={type}
        className="size-7 shrink-0 text-accent-orange/70 transition-colors group-hover:text-accent-orange"
      />
      <span className="font-medium text-foreground">{adapterLabel(type)}</span>
    </div>
    <Badge
      variant="outline"
      className="shrink-0 rounded-md px-3 py-1 transition-colors group-hover:border-[var(--accent-orange)] group-hover:text-[var(--accent-orange)]"
    >
      USE
    </Badge>
  </button>
)
