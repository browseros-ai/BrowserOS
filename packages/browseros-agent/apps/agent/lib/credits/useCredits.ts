import { EXTERNAL_URLS } from '@browseros/shared/constants/urls'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { getBrowserosId } from './browseros-id'

export interface CreditsInfo {
  credits: number
  dailyLimit: number
  lastResetAt?: string
  browserosId?: string
}

const CREDITS_QUERY_KEY = ['credits']

async function fetchCredits(): Promise<CreditsInfo> {
  const browserosId = await getBrowserosId()
  const response = await fetch(
    `${EXTERNAL_URLS.CREDITS_GATEWAY}/credits/${browserosId}`,
  )
  if (!response.ok)
    throw new Error(`Failed to fetch credits: ${response.status}`)
  const data = (await response.json()) as CreditsInfo
  return { ...data, browserosId }
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
