import { Loader2, Plus } from 'lucide-react'
import type { FC } from 'react'
import { Button } from '@/components/ui/button'
import { HubProviderRow } from './HubProviderRow'
import type { LlmHubProvider } from './models'

export interface HubProvidersListProps {
  providers: LlmHubProvider[]
  isLoading?: boolean
  onEditProvider: (index: number) => void
  onDeleteProvider: (index: number) => void
  onAddProvider: () => void
}

export const HubProvidersList: FC<HubProvidersListProps> = ({
  providers,
  isLoading = false,
  onEditProvider,
  onDeleteProvider,
  onAddProvider,
}) => {
  if (isLoading) {
    return (
      <div className="rounded-xl border border-border bg-card p-6 shadow-sm">
        <div className="flex items-center justify-center gap-3 py-8 text-muted-foreground">
          <Loader2 className="h-5 w-5 animate-spin" />
          <span>Loading providers...</span>
        </div>
      </div>
    )
  }

  if (providers.length === 0) {
    return (
      <div className="rounded-xl border border-border bg-card p-6 shadow-sm">
        <div className="mb-4 flex items-center justify-between">
          <h3 className="font-medium">Configured AI Providers</h3>
          <Button
            variant="outline"
            size="sm"
            onClick={onAddProvider}
            className="border-[var(--accent-orange)] bg-[var(--accent-orange)]/10 text-[var(--accent-orange)] hover:bg-[var(--accent-orange)]/20 hover:text-[var(--accent-orange)]"
          >
            <Plus className="mr-1.5 h-4 w-4" />
            Add provider
          </Button>
        </div>
        <div className="rounded-lg border border-border border-dashed py-8 text-center">
          <p className="text-muted-foreground text-sm">
            No providers configured yet.
          </p>
        </div>
      </div>
    )
  }

  return (
    <div className="rounded-xl border border-border bg-card p-6 shadow-sm">
      <div className="mb-4 flex items-center justify-between">
        <h3 className="font-medium">Configured AI Providers</h3>
        <Button
          variant="outline"
          size="sm"
          onClick={onAddProvider}
          className="border-[var(--accent-orange)] bg-[var(--accent-orange)]/10 text-[var(--accent-orange)] hover:bg-[var(--accent-orange)]/20 hover:text-[var(--accent-orange)]"
        >
          <Plus className="mr-1.5 h-4 w-4" />
          Add provider
        </Button>
      </div>
      <div className="space-y-3">
        {providers.map((provider, index) => (
          <HubProviderRow
            key={provider.url}
            provider={provider}
            canDelete={providers.length > 1}
            onEdit={() => onEditProvider(index)}
            onDelete={() => onDeleteProvider(index)}
          />
        ))}
      </div>
    </div>
  )
}
