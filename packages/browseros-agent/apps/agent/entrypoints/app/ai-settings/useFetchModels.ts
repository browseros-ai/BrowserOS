import { useState } from 'react'
import {
  type FetchModelsResult,
  fetchModelsFromApi,
} from '@/lib/llm-providers/fetchModels'

interface UseFetchModelsReturn {
  isFetching: boolean
  result: FetchModelsResult | null
  fetch: (baseUrl: string, apiKey?: string) => Promise<FetchModelsResult>
  reset: () => void
}

export function useFetchModels(): UseFetchModelsReturn {
  const [isFetching, setIsFetching] = useState(false)
  const [result, setResult] = useState<FetchModelsResult | null>(null)

  const fetch = async (baseUrl: string, apiKey?: string) => {
    setIsFetching(true)
    setResult(null)
    try {
      const res = await fetchModelsFromApi(baseUrl, apiKey)
      setResult(res)
      return res
    } finally {
      setIsFetching(false)
    }
  }

  const reset = () => {
    setResult(null)
  }

  return { isFetching, result, fetch, reset }
}
