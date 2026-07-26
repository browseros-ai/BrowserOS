import { EXTERNAL_URLS } from '@browseros/shared/constants/urls'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useCallback } from 'react'
import { getBrowserosId } from '@/lib/credits/browseros-id'

interface CreditsInfo {
  credits: number
  dailyLimit: number
  lastResetAt?: string
  browserosId?: string
}

const CREDITS_QUERY_KEY = ['credits']
const CREDITS_EXHAUSTED_OVERRIDE_KEY = 'shimmy-credits-exhausted-until'

function getNextUtcMidnightTimestamp(now = new Date()): number {
  return Date.UTC(
    now.getUTCFullYear(),
    now.getUTCMonth(),
    now.getUTCDate() + 1,
    0,
    0,
    0,
    0,
  )
}

function readCreditsExhaustedOverride(): number | null {
  if (typeof window === 'undefined') return null

  const raw = window.localStorage.getItem(CREDITS_EXHAUSTED_OVERRIDE_KEY)
  if (!raw) return null

  const expiresAt = Number(raw)
  if (!Number.isFinite(expiresAt) || expiresAt <= Date.now()) {
    window.localStorage.removeItem(CREDITS_EXHAUSTED_OVERRIDE_KEY)
    return null
  }

  return expiresAt
}

function writeCreditsExhaustedOverride(expiresAt: number) {
  if (typeof window === 'undefined') return
  window.localStorage.setItem(CREDITS_EXHAUSTED_OVERRIDE_KEY, String(expiresAt))
}

function applyCreditsExhaustedOverride(data: CreditsInfo): CreditsInfo {
  const overrideExpiresAt = readCreditsExhaustedOverride()
  if (!overrideExpiresAt) return data
  return { ...data, credits: 0 }
}

async function fetchCredits(): Promise<CreditsInfo> {
  const browserosId = await getBrowserosId()
  const response = await fetch(
    `${EXTERNAL_URLS.CREDITS_GATEWAY}/credits/${browserosId}`,
  )
  if (!response.ok)
    throw new Error(`Failed to fetch credits: ${response.status}`)
  const data = (await response.json()) as CreditsInfo
  return applyCreditsExhaustedOverride({ ...data, browserosId })
}

export function useCredits() {
  return useQuery<CreditsInfo>({
    queryKey: CREDITS_QUERY_KEY,
    queryFn: fetchCredits,
    refetchOnWindowFocus: true,
    refetchOnMount: 'always',
    staleTime: 30_000,
    retry: 1,
  })
}

export function useInvalidateCredits() {
  const queryClient = useQueryClient()
  return useCallback(
    () => queryClient.invalidateQueries({ queryKey: CREDITS_QUERY_KEY }),
    [queryClient],
  )
}

export function useMarkCreditsExhausted() {
  const queryClient = useQueryClient()

  return useCallback(() => {
    const expiresAt = getNextUtcMidnightTimestamp()
    writeCreditsExhaustedOverride(expiresAt)

    queryClient.setQueryData<CreditsInfo>(CREDITS_QUERY_KEY, (current) => {
      if (!current) {
        return {
          credits: 0,
          dailyLimit: 50,
        }
      }

      return {
        ...current,
        credits: 0,
      }
    })
  }, [queryClient])
}
