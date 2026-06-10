import { Plus } from 'lucide-react'
import type { FC } from 'react'
import ProductLogoSvg from '@/assets/product_logo.svg'
import { Button } from '@/components/ui/button'
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { HarnessAgent } from '@/modules/agents/agent-harness-types'
import type { SidepanelChatTargetSelection } from '@/modules/chat/sidepanel-chat-targets'
import {
  decodeTargetValue,
  encodeTargetValue,
} from './default-chat-target.helpers'

interface LlmProvidersHeaderProps {
  providers: LlmProviderConfig[]
  agents: HarnessAgent[]
  selectedTarget: SidepanelChatTargetSelection
  onSelectTarget: (selection: SidepanelChatTargetSelection) => void
  onAddProvider: () => void
}

/**
 * Header section for LLM providers with the default-target selector (LLM
 * providers and coding agents) and add button.
 */
export const LlmProvidersHeader: FC<LlmProvidersHeaderProps> = ({
  providers,
  agents,
  selectedTarget,
  onSelectTarget,
  onAddProvider,
}) => {
  return (
    <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
      <div className="flex items-start gap-4">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-orange)]/10">
          <img src={ProductLogoSvg} alt="BrowserOS" className="h-8 w-8" />
        </div>
        <div className="flex-1">
          <h2 className="mb-1 font-semibold text-xl">LLM Providers</h2>
          <p className="mb-6 text-muted-foreground text-sm">
            Add your provider and choose the default for new chats
          </p>

          <div className="flex flex-col items-start gap-4 sm:flex-row sm:items-center">
            <label
              htmlFor="provider-picker"
              className="whitespace-nowrap font-medium text-sm"
            >
              Default Provider:
            </label>
            <Select
              value={encodeTargetValue(selectedTarget)}
              onValueChange={(value) => {
                const selection = decodeTargetValue(value)
                if (selection) onSelectTarget(selection)
              }}
            >
              <SelectTrigger
                id="provider-picker"
                className="w-full flex-1 sm:max-w-xs"
              >
                <SelectValue placeholder="Select a provider" />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectLabel>LLM providers</SelectLabel>
                  {providers.map((provider) => (
                    <SelectItem
                      key={provider.id}
                      value={encodeTargetValue({
                        kind: 'llm',
                        id: provider.id,
                      })}
                    >
                      {provider.name}
                    </SelectItem>
                  ))}
                </SelectGroup>
                {agents.length > 0 && (
                  <SelectGroup>
                    <SelectLabel>Coding agents</SelectLabel>
                    {agents.map((agent) => (
                      <SelectItem
                        key={agent.id}
                        value={encodeTargetValue({ kind: 'acp', id: agent.id })}
                      >
                        {agent.name}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                )}
              </SelectContent>
            </Select>
            <Button
              variant="outline"
              onClick={onAddProvider}
              className="border-[var(--accent-orange)] bg-[var(--accent-orange)]/10 text-[var(--accent-orange)] hover:bg-[var(--accent-orange)]/20 hover:text-[var(--accent-orange)]"
            >
              <Plus className="h-4 w-4" />
              Add custom provider
            </Button>
          </div>
        </div>
      </div>
    </div>
  )
}
