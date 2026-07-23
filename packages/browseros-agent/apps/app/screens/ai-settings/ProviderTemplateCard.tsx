import type { FC } from 'react'
import { Badge } from '@/components/ui/badge'
import { ProviderIcon } from '@/lib/llm-providers/providerIcons'
import type { ProviderTemplate } from '@/lib/llm-providers/providerTemplates'

export interface ProviderTemplateCardProps {
  template: ProviderTemplate
  onUseTemplate: (template: ProviderTemplate) => void
}

export const ProviderTemplateCard: FC<ProviderTemplateCardProps> = ({
  template,
  onUseTemplate,
}) => {
  return (
    <button
      type="button"
      onClick={() => onUseTemplate(template)}
      className="group relative flex w-full items-center gap-3 rounded-lg border border-border bg-background p-4 text-left transition-all hover:border-[var(--accent-orange)] hover:shadow-md"
    >
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <ProviderIcon
          type={template.id}
          size={28}
          className="shrink-0 text-accent-orange/70 transition-colors group-hover:text-accent-orange"
        />
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium text-foreground">{template.name}</span>
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
