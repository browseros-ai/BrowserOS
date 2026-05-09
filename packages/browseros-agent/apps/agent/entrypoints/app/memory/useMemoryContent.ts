import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'

export const MEMORY_QUERY_KEY = 'memory'

async function fetchMemory(baseUrl: string): Promise<string> {
  const response = await fetch(`${baseUrl}/memory`)
  if (!response.ok) throw new Error(`HTTP ${response.status}`)
  const data = await response.json()
  return data.content || ''
}

async function saveMemory(baseUrl: string, content: string): Promise<void> {
  const response = await fetch(`${baseUrl}/memory`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ content }),
  })
  if (!response.ok) {
    let message = `HTTP ${response.status}`
    try {
      const body = await response.json()
      if (body?.error?.message) message = body.error.message
      else if (body?.success === false && body?.message) message = body.message
    } catch {}
    throw new Error(message)
  }
}

export function useMemoryContent() {
  const { baseUrl, isLoading: urlLoading } = useAgentServerUrl()
  const queryClient = useQueryClient()

  const { data, isLoading, error, refetch } = useQuery<string, Error>({
    queryKey: [MEMORY_QUERY_KEY, baseUrl],
    queryFn: () => fetchMemory(baseUrl as string),
    enabled: !!baseUrl && !urlLoading,
  })

  const saveMutation = useMutation({
    mutationFn: (content: string) => saveMemory(baseUrl as string, content),
    onSuccess: (_data, content) => {
      queryClient.setQueryData([MEMORY_QUERY_KEY, baseUrl], content)
    },
  })

  return {
    content: data ?? null,
    isLoading: isLoading || urlLoading,
    error,
    refetch,
    save: saveMutation.mutateAsync,
    isSaving: saveMutation.isPending,
    saveError: saveMutation.error,
    resetSaveError: saveMutation.reset,
  }
}
