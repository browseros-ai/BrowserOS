import type { FC } from 'react'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { ProviderCard } from './ProviderCard'

interface ConfiguredProvidersListProps {
  providers: LlmProviderConfig[]
  selectedProviderId: string | null
  testingProviderId: string | null
  onSelectProvider: (providerId: string) => void
  onTestProvider: (provider: LlmProviderConfig) => void
  onEditProvider: (provider: LlmProviderConfig) => void
  onDeleteProvider: (provider: LlmProviderConfig) => void
}

/**
 * List of configured LLM providers with selection capability
 */
export const ConfiguredProvidersList: FC<ConfiguredProvidersListProps> = ({
  providers,
  selectedProviderId,
  testingProviderId,
  onSelectProvider,
  onTestProvider,
  onEditProvider,
  onDeleteProvider,
}) => {
  return (
    <div className="space-y-3">
      {providers.map((provider) => {
        const isBuiltIn = provider.id === 'browseros'

        return (
          <ProviderCard
            key={provider.id}
            provider={provider}
            isSelected={selectedProviderId === provider.id}
            isBuiltIn={isBuiltIn}
            isTesting={testingProviderId === provider.id}
            onSelect={() => onSelectProvider(provider.id)}
            onTest={() => onTestProvider(provider)}
            onEdit={() => onEditProvider(provider)}
            onDelete={() => onDeleteProvider(provider)}
          />
        )
      })}
    </div>
  )
}
