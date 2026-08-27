import { Plus } from 'lucide-react'
import { type FC, useId, useState } from 'react'
import { AdapterIcon, adapterLabel } from '@/components/agents/AdapterIcon'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Feature } from '@/lib/browseros/capabilities'
import { ProviderIcon } from '@/lib/llm-providers/providerIcons'
import {
  type ProviderTemplate,
  providerTemplates,
} from '@/lib/llm-providers/providerTemplates'
import type { AcpAgentType } from '@/modules/agents/acp-agent-types'
import { useCapabilities } from '@/modules/browseros/capabilities.hooks'
import { AddProviderTile } from './AddProviderTile'
import {
  type AddProviderEntryMeta,
  categoryForTemplate,
  groupAddProviderEntries,
} from './add-provider.helpers'
import { CustomAgentTile } from './CustomAgentTile'
import { POPULAR_ACP_AGENTS } from './popular-acp-agents'

export interface AddProviderSectionProps {
  onCreateAgent: (type: AcpAgentType) => void
  onCreateCustomAgent: () => void
  onUseTemplate: (template: ProviderTemplate) => void
}

interface Entry extends AddProviderEntryMeta {
  icon: React.ReactNode
  onAdd: () => void
  /** Rendered as the wide tile rather than one cell of the grid. */
  featured?: boolean
}

const AGENT_TYPES = ['claude', 'codex'] as const

export const AddProviderSection: FC<AddProviderSectionProps> = ({
  onCreateAgent,
  onCreateCustomAgent,
  onUseTemplate,
}) => {
  const searchId = useId()
  const [query, setQuery] = useState('')
  const { supports } = useCapabilities()
  const supportsCodingAgents = supports(Feature.AGENT_HARNESS_SUPPORT)

  const templateEntries: Entry[] = providerTemplates
    .filter((template) => {
      if (template.id === 'chatgpt-pro')
        return supports(Feature.CHATGPT_PRO_SUPPORT)
      if (template.id === 'github-copilot')
        return supports(Feature.GITHUB_COPILOT_SUPPORT)
      if (template.id === 'qwen-code')
        return supports(Feature.QWEN_CODE_SUPPORT)
      return true
    })
    .map((template) => ({
      key: `template-${template.id}`,
      label: template.name,
      category: categoryForTemplate(template.id),
      icon: <ProviderIcon type={template.id} size={26} />,
      onAdd: () => onUseTemplate(template),
    }))

  const agentEntries: Entry[] = supportsCodingAgents
    ? [
        ...AGENT_TYPES.map((type) => ({
          key: `agent-${type}`,
          label: adapterLabel(type),
          category: 'agent' as const,
          icon: <AdapterIcon adapter={type} className="size-[26px]" />,
          onAdd: () => onCreateAgent(type),
        })),
        {
          key: 'agent-custom',
          label: 'Custom ACP agent',
          category: 'agent' as const,
          keywords: POPULAR_ACP_AGENTS.flatMap((agent) => [
            agent.id,
            agent.label,
          ]),
          icon: <Plus className="size-[26px]" />,
          onAdd: onCreateCustomAgent,
          featured: true,
        },
      ]
    : []

  const groups = groupAddProviderEntries(
    [...agentEntries, ...templateEntries],
    query,
  )

  return (
    <section className="space-y-4">
      <h3 className="font-semibold text-base">Add a provider</h3>

      <div className="space-y-1.5">
        <Label htmlFor={searchId}>Search providers and agents</Label>
        <Input
          id={searchId}
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Try claude, ollama, or openai"
        />
      </div>

      {groups.length === 0 ? (
        <p className="text-muted-foreground text-sm">
          No provider matches that search.
        </p>
      ) : (
        groups.map((group) => (
          <div key={group.category} className="space-y-2">
            <h4 className="font-semibold text-muted-foreground text-xs uppercase tracking-wider">
              {group.label}
            </h4>
            <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
              {group.entries.map((entry) =>
                entry.featured ? (
                  <CustomAgentTile key={entry.key} onAdd={entry.onAdd} />
                ) : (
                  <AddProviderTile
                    key={entry.key}
                    label={entry.label}
                    icon={entry.icon}
                    onAdd={entry.onAdd}
                  />
                ),
              )}
            </div>
          </div>
        ))
      )}
    </section>
  )
}
