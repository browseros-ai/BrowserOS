import { KeyRound, Trash2 } from 'lucide-react'
import type { FC } from 'react'
import { Button } from '@/components/ui/button'
import { ProviderIcon } from '@/lib/llm-providers/providerIcons'
import type { ProviderType } from '@/lib/llm-providers/types'

export interface IncompleteProvider {
  rowId: string
  type: string
  name: string
  baseUrl?: string | null
  modelId: string
  supportsImages: boolean
  contextWindow?: number | null
  temperature?: number | null
  resourceName?: string | null
  region?: string | null
}

export interface IncompleteProviderCardProps {
  provider: IncompleteProvider
  onAddKeys: () => void
  onDelete: () => void
}

export const IncompleteProviderCard: FC<IncompleteProviderCardProps> = ({
  provider,
  onAddKeys,
  onDelete,
}) => {
  return (
    <div className="flex w-full items-center gap-4 rounded-xl border border-amber-500/50 border-dashed bg-amber-500/5 p-4">
      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-amber-500/10 text-amber-500">
        <ProviderIcon type={provider.type as ProviderType} size={24} />
      </div>
      <div className="min-w-0 flex-1">
        <div className="mb-1 flex items-center gap-2">
          <span className="font-semibold">{provider.name}</span>
        </div>
        <p className="truncate text-muted-foreground text-sm">
          {provider.modelId}
          {provider.baseUrl && ` • ${provider.baseUrl}`}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Button variant="outline" size="sm" onClick={onAddKeys}>
          <KeyRound className="mr-1.5 h-4 w-4" />
          Add Keys
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
          onClick={onDelete}
        >
          <Trash2 className="h-4 w-4" />
        </Button>
      </div>
    </div>
  )
}
