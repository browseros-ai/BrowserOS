import { Loader2 } from 'lucide-react'
import { type FC, useCallback } from 'react'
import { Button } from '@/components/ui/button'
import { useFetchModels } from './useFetchModels'

interface FetchModelsButtonProps {
  baseUrl: string
  apiKey?: string
  onFetchComplete: (models: string[]) => void
  disabled?: boolean
}

export const FetchModelsButton: FC<FetchModelsButtonProps> = ({
  baseUrl,
  apiKey,
  onFetchComplete,
  disabled = false,
}) => {
  const { isFetching, result, fetch } = useFetchModels()

  const handleClick = useCallback(async () => {
    const res = await fetch(baseUrl, apiKey)
    if (res.success) {
      const modelIds = res.models.map((m) => m.id)
      onFetchComplete(modelIds)
    }
  }, [baseUrl, apiKey, fetch, onFetchComplete])

  return (
    <div className="space-y-1">
      <Button
        type="button"
        variant="outline"
        size="sm"
        onClick={handleClick}
        disabled={disabled || isFetching}
      >
        {isFetching ? (
          <>
            <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
            Fetching...
          </>
        ) : (
          <>🔄 Fetch Models from API</>
        )}
      </Button>
      {result && !result.success && result.error && (
        <p className="text-destructive text-xs">
          Failed to fetch: {result.error}
        </p>
      )}
    </div>
  )
}
