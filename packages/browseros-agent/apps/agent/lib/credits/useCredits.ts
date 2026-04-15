import { useQuery, useQueryClient } from '@tanstack/react-query'
import { getAgentServerUrl } from '@/lib/browseros/helpers'

export interface CreditsInfo {
  credits: number
  dailyLimit: number
  lastResetAt?: string
  browserosId?: string
}

const CREDITS_QUERY_KEY = ['credits']

async function fetchCredits(): Promise<CreditsInfo> {
  const baseUrl = await getAgentServerUrl()
  const response = await fetch(`${baseUrl}/credits`)
  if (!response.ok)
    throw new Error(`Failed to fetch credits: ${response.status}`)
  return response.json()
}

export function useCredits() {
  return useQuery<CreditsInfo>({
    queryKey: CREDITS_QUERY_KEY,
    queryFn: fetchCredits,
    refetchOnWindowFocus: true,
    staleTime: 30_000,
    retry: 1,
  })
}

export function useInvalidateCredits() {
  const queryClient = useQueryClient()
  return () => queryClient.invalidateQueries({ queryKey: CREDITS_QUERY_KEY })
}
