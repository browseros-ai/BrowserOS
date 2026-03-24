import { useQuery } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useRpcClient } from '@/lib/rpc/RpcClientProvider'

export const SOUL_QUERY_KEY = 'soul'
let soulCache: string | null = null

export function useSoulContent() {
  const rpcClient = useRpcClient()

  const { data, isLoading, error, refetch } = useQuery<string, Error>({
    queryKey: [SOUL_QUERY_KEY],
    queryFn: async () => {
      const response = await rpcClient.soul.$get()
      if (!response.ok) throw new Error(`HTTP ${response.status}`)
      const result = await response.json()
      return result.content || ''
    },
    placeholderData: () => soulCache ?? undefined,
  })

  useEffect(() => {
    if (data === undefined) return
    soulCache = data
  }, [data])

  const content = data ?? soulCache ?? null

  return {
    content,
    isLoading: isLoading && content === null,
    error,
    refetch,
  }
}
