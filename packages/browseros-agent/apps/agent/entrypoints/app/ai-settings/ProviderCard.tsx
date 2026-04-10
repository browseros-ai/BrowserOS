import { Check, Loader2, Trash2 } from 'lucide-react'
import type { FC } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { BrowserOSIcon, ProviderIcon } from '@/lib/llm-providers/providerIcons'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { cn } from '@/lib/utils'

interface ProviderCardProps {
  provider: LlmProviderConfig
  isSelected: boolean
  isBuiltIn: boolean
  onSelect: () => void
  onTest?: () => void
  onEdit?: () => void
  onDelete?: () => void
  isTesting?: boolean
}

/** Card component for displaying a configured LLM provider */
export const ProviderCard: FC<ProviderCardProps> = ({
  provider,
  isSelected,
  isBuiltIn,
  onSelect,
  onTest,
  onEdit,
  onDelete,
  isTesting = false,
}) => {
  const inputId = `provider-${provider.id}`

  return (
    <label
      htmlFor={inputId}
      className={cn(
        'group flex w-full cursor-pointer items-center gap-4 rounded-xl border p-4 text-left transition-all',
        isSelected
          ? 'border-[var(--accent-orange)] bg-[var(--accent-orange)]/5 shadow-md'
          : 'border-border bg-card hover:border-[var(--accent-orange)]/50 hover:shadow-sm',
      )}
    >
      <input
        type="radio"
        id={inputId}
        name="default-provider"
        className="sr-only"
        checked={isSelected}
        onChange={() => onSelect()}
      />
      <div
        className={cn(
          'flex h-5 w-5 shrink-0 items-center justify-center rounded-full border-2 transition-all',
          isSelected
            ? 'border-[var(--accent-orange)] bg-[var(--accent-orange)]'
            : 'border-border',
        )}
      >
        {isSelected && <Check className="h-3 w-3 text-white" />}
      </div>
      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-[var(--accent-orange)]/10 text-[var(--accent-orange)]">
        {isBuiltIn ? (
          <BrowserOSIcon size={24} />
        ) : (
          <ProviderIcon type={provider.type} size={24} />
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="mb-1 flex items-center gap-2">
          <span className="font-semibold">{provider.name}</span>
          {isSelected && (
            <Badge
              variant="secondary"
              className="rounded bg-[var(--accent-orange)]/10 text-[var(--accent-orange)]"
            >
              DEFAULT
            </Badge>
          )}
        </div>
        <p className="truncate text-muted-foreground text-sm">
          {isBuiltIn ? (
            <>
              BrowserOS-hosted model with strict rate limits.{' '}
              <a
                href="https://docs.browseros.com/features/bring-your-own-llm"
                target="_blank"
                rel="noopener noreferrer"
                className="underline hover:text-foreground"
                onClick={(e) => e.stopPropagation()}
              >
                Bring your own key
              </a>{' '}
              for better performance.
            </>
          ) : provider.baseUrl ? (
            `${provider.modelId} • ${provider.baseUrl}`
          ) : (
            provider.modelId
          )}
        </p>
      </div>
      {!isBuiltIn && (
        <div className="flex shrink-0 items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={isTesting}
            onClick={() => onTest?.()}
          >
            {isTesting && <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />}
            {isTesting ? 'Testing...' : 'Test'}
          </Button>
          <Button variant="outline" size="sm" onClick={() => onEdit?.()}>
            Edit
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
            onClick={() => onDelete?.()}
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      )}
    </label>
  )
}
