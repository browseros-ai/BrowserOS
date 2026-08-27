import { Plus } from 'lucide-react'
import type { FC } from 'react'
import { Badge } from '@/components/ui/badge'

export interface CustomAgentTemplateCardProps {
  onCreate: () => void
}

export const CustomAgentTemplateCard: FC<CustomAgentTemplateCardProps> = ({
  onCreate,
}) => (
  <button
    type="button"
    onClick={onCreate}
    className="group relative flex w-full items-center gap-3 rounded-lg border border-[var(--accent-orange)]/40 border-dashed bg-background p-4 text-left transition-all hover:border-[var(--accent-orange)] hover:shadow-md"
  >
    <div className="flex min-w-0 flex-1 items-center gap-3">
      <Plus className="size-7 shrink-0 text-accent-orange/70 transition-colors group-hover:text-accent-orange" />
      <span className="font-medium text-foreground">Custom ACP agent</span>
    </div>
    <Badge
      variant="outline"
      className="shrink-0 rounded-md px-3 py-1 transition-colors group-hover:border-[var(--accent-orange)] group-hover:text-[var(--accent-orange)]"
    >
      ADD
    </Badge>
  </button>
)
