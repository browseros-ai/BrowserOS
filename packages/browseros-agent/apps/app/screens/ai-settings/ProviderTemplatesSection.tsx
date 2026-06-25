import { ChevronDown } from 'lucide-react'
import { type FC, Fragment } from 'react'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { Feature } from '@/lib/browseros/capabilities'
import { visibleProviderTemplates } from '@/lib/llm-providers/provider-visibility'
import {
  type ProviderTemplate,
  providerTemplates,
} from '@/lib/llm-providers/providerTemplates'
import { REMOTE_HERMES_PROVIDER_TYPE } from '@/lib/llm-providers/types'
import { cn } from '@/lib/utils'
import type {
  HarnessAdapterDescriptor,
  HarnessAgentAdapter,
} from '@/modules/agents/agent-harness-types'
import { useCapabilities } from '@/modules/browseros/capabilities.hooks'
import { CodingAgentTemplateCard } from './CodingAgentTemplateCard'
import { ProviderTemplateCard } from './ProviderTemplateCard'

export interface ProviderTemplatesSectionProps {
  /** Coding-agent runtimes (Claude Code / Codex) shown first in the grid. */
  codingAdapters: HarnessAdapterDescriptor[]
  onCreateAgent: (adapterId: HarnessAgentAdapter) => void
  onUseTemplate: (template: ProviderTemplate) => void
}

export const ProviderTemplatesSection: FC<ProviderTemplatesSectionProps> = ({
  codingAdapters,
  onCreateAgent,
  onUseTemplate,
}) => {
  const { supports } = useCapabilities()

  const filteredTemplates = visibleProviderTemplates(
    providerTemplates,
    supports,
  )
    .filter((template) => {
      if (template.id === 'chatgpt-pro')
        return supports(Feature.CHATGPT_PRO_SUPPORT)
      if (template.id === 'github-copilot')
        return supports(Feature.GITHUB_COPILOT_SUPPORT)
      if (template.id === 'qwen-code')
        return supports(Feature.QWEN_CODE_SUPPORT)
      return true
    })
    .sort((a, b) => {
      if (a.id === REMOTE_HERMES_PROVIDER_TYPE) return -1
      if (b.id === REMOTE_HERMES_PROVIDER_TYPE) return 1
      return 0
    })

  return (
    <Collapsible defaultOpen className="group/collapsible">
      <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
        <CollapsibleTrigger className="mb-4 flex w-full items-center justify-between text-left">
          <div>
            <h3 className="font-semibold text-lg">Quick provider templates</h3>
            <p className="text-muted-foreground text-sm">
              {codingAdapters.length + filteredTemplates.length} templates
              available
            </p>
          </div>
          <ChevronDown
            className={cn(
              'h-5 w-5 transition-transform',
              'group-data-[state=open]/collapsible:rotate-180',
            )}
          />
        </CollapsibleTrigger>

        <CollapsibleContent>
          <div className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {filteredTemplates.map((template, idx) => {
              const isHermes = template.id === REMOTE_HERMES_PROVIDER_TYPE
              const showCodingAdapters = idx === 0
              return (
                <Fragment key={template.id}>
                  <ProviderTemplateCard
                    template={template}
                    highlighted={isHermes}
                    onUseTemplate={onUseTemplate}
                  />
                  {showCodingAdapters &&
                    codingAdapters.map((adapter) => (
                      <CodingAgentTemplateCard
                        key={`coding-${adapter.id}`}
                        adapter={adapter}
                        onCreate={onCreateAgent}
                      />
                    ))}
                </Fragment>
              )
            })}
          </div>
        </CollapsibleContent>
      </div>
    </Collapsible>
  )
}
