import type { FC } from 'react'
import { Badge } from '@/components/ui/badge'
import { ProviderIcon } from '@/lib/llm-providers/providerIcons'
import type { ProviderTemplate } from '@/lib/llm-providers/providerTemplates'
import { cn } from '@/lib/utils'

interface ProviderTemplateCardProps {
  template: ProviderTemplate
  highlighted?: boolean
  isNew?: boolean
  onUseTemplate: (template: ProviderTemplate) => void
}

export const ProviderTemplateCard: FC<ProviderTemplateCardProps> = ({
  template,
  highlighted = false,
  isNew = false,
  onUseTemplate,
}) => {
  return (
    <button
      type="button"
      onClick={() => onUseTemplate(template)}
      className={cn(
        'group pointer-events-auto relative z-10 flex w-full cursor-pointer items-center gap-3 rounded-lg border bg-background p-4 text-left transition-all hover:border-[var(--accent-orange)] hover:shadow-md',
        highlighted
          ? 'border-stone-400/45 bg-stone-100/50 shadow-sm ring-1 ring-stone-400/35 dark:bg-stone-500/10'
          : isNew
            ? 'border-2 border-[var(--accent-orange)]/50'
            : 'border-border',
      )}
    >
      {isNew && (
        <span className="absolute -top-2 left-3 rounded-full bg-[var(--accent-orange)] px-2 py-0.5 font-semibold text-[9px] text-white uppercase tracking-wider">
          New
        </span>
      )}
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <ProviderIcon
          type={template.id}
          size={28}
          className="shrink-0 text-accent-orange/70 transition-colors group-hover:text-accent-orange"
        />
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium text-foreground">{template.name}</span>
            {highlighted && (
              <span className="rounded-full border border-stone-400/50 bg-stone-200/60 px-2 py-0.5 font-semibold text-[10px] text-stone-700 dark:border-stone-500/40 dark:bg-stone-600/25 dark:text-stone-200">
                Recommended
              </span>
            )}
          </div>
        </div>
      </div>
      <Badge
        variant="outline"
        className={cn(
          'shrink-0 rounded-md px-3 py-1 transition-colors group-hover:border-[var(--accent-orange)] group-hover:text-[var(--accent-orange)]',
          highlighted &&
            'border-[var(--accent-orange)] bg-[var(--accent-orange)]/5 text-[var(--accent-orange)]',
        )}
      >
        USE
      </Badge>
    </button>
  )
}
