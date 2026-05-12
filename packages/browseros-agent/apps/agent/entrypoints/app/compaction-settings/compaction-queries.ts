import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'

export interface VccConfig {
  maxTranscriptLines?: number
  maxGoalLines?: number
  maxFileEntries?: number
  maxCommitEntries?: number
  maxPreferenceLines?: number
  maxOutstandingLines?: number
}

export interface CompactionConfig {
  method: 'default' | 'vcc'
  customPrompt?: string
  vccConfig?: VccConfig
}

export interface CompactionConfigResponse {
  active: CompactionConfig | null
  defaults: { method: 'default' }
}

export interface CompactionSaveResponse {
  ok: boolean
  saved?: CompactionConfig
  errors?: Array<{ key: string; message: string }>
}

const COMPACTION_QUERY_KEY = ['compaction-config'] as const

async function fetchCompactionConfig(
  baseUrl: string,
): Promise<CompactionConfigResponse> {
  const res = await fetch(`${baseUrl}/compaction`)
  if (!res.ok) throw new Error(`HTTP ${res.status}`)
  return res.json()
}

async function putCompactionConfig(
  baseUrl: string,
  config: CompactionConfig,
): Promise<CompactionSaveResponse> {
  const res = await fetch(`${baseUrl}/compaction`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(config),
  })
  return res.json()
}

async function deleteCompactionConfig(
  baseUrl: string,
): Promise<CompactionSaveResponse> {
  const res = await fetch(`${baseUrl}/compaction`, { method: 'DELETE' })
  return res.json()
}

export function useCompactionConfig() {
  const {
    baseUrl,
    isLoading: urlLoading,
    error: urlError,
  } = useAgentServerUrl()
  const queryClient = useQueryClient()

  const query = useQuery<CompactionConfigResponse, Error>({
    queryKey: [...COMPACTION_QUERY_KEY, baseUrl],
    queryFn: () => fetchCompactionConfig(baseUrl as string),
    enabled: !!baseUrl && !urlLoading,
  })

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: COMPACTION_QUERY_KEY })

  const saveMutation = useMutation({
    mutationFn: (config: CompactionConfig) =>
      putCompactionConfig(baseUrl as string, config),
    onSuccess: invalidate,
  })

  const resetMutation = useMutation({
    mutationFn: () => deleteCompactionConfig(baseUrl as string),
    onSuccess: invalidate,
  })

  return {
    config: query.data ?? null,
    isLoading: query.isLoading || urlLoading,
    error: query.error ?? urlError,
    refetch: query.refetch,
    saveConfig: saveMutation.mutateAsync,
    resetConfig: resetMutation.mutateAsync,
    isSaving: saveMutation.isPending,
    isResetting: resetMutation.isPending,
  }
}
