import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'

export interface ConfigKeySchema {
  key: string
  label: string
  group: 'Timeouts' | 'Limits' | 'Retention'
  section: 'safe' | 'dangerous'
  unit: 'ms' | 'turns' | 'tokens' | 'chars' | 'lines' | 'days'
  min: number
  max: number
  default: number
  description: string
  risk?: string
  envVar: string
}

export interface ConfigResponse {
  active: Record<string, number>
  pending: Record<string, number>
  defaults: Record<string, number>
  schema: Record<string, ConfigKeySchema>
  hasPendingChanges: boolean
}

export interface ConfigSaveResponse {
  ok: boolean
  saved?: number
  hasPendingChanges?: boolean
  errors?: Array<{ key: string; message: string }>
}

const CONFIG_QUERY_KEY = ['advanced-config'] as const

async function fetchConfig(baseUrl: string): Promise<ConfigResponse> {
  const res = await fetch(`${baseUrl}/config`)
  if (!res.ok) throw new Error(`HTTP ${res.status}`)
  return res.json()
}

async function putConfig(
  baseUrl: string,
  overrides: Record<string, number>,
): Promise<ConfigSaveResponse> {
  const res = await fetch(`${baseUrl}/config`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ overrides }),
  })
  return res.json()
}

async function deleteConfig(baseUrl: string): Promise<ConfigSaveResponse> {
  const res = await fetch(`${baseUrl}/config`, { method: 'DELETE' })
  return res.json()
}

export function useAdvancedConfig() {
  const {
    baseUrl,
    isLoading: urlLoading,
    error: urlError,
  } = useAgentServerUrl()
  const queryClient = useQueryClient()

  const query = useQuery<ConfigResponse, Error>({
    queryKey: [...CONFIG_QUERY_KEY, baseUrl],
    queryFn: () => fetchConfig(baseUrl as string),
    enabled: !!baseUrl && !urlLoading,
  })

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: CONFIG_QUERY_KEY })

  const saveMutation = useMutation({
    mutationFn: (overrides: Record<string, number>) =>
      putConfig(baseUrl as string, overrides),
    onSuccess: invalidate,
  })

  const resetMutation = useMutation({
    mutationFn: () => deleteConfig(baseUrl as string),
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
