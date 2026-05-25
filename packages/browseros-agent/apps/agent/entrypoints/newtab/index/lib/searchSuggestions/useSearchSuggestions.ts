import { useEffect, useState } from 'react'
import useSWR from 'swr'
import { getSearchSuggestions } from './getSearchSuggestions'

interface UseSearchSuggestionsArgs {
  query: string
  debounceMs?: number
}

/**
 * Debounces new-tab search query changes and loads Google suggestions through SWR.
 */
export const useSearchSuggestions = ({
  query,
  debounceMs = 300,
}: UseSearchSuggestionsArgs) => {
  const [debouncedQuery, setDebouncedQuery] = useState(query)

  useEffect(() => {
    const timer = setTimeout(() => {
      setDebouncedQuery(query)
    }, debounceMs)

    return () => clearTimeout(timer)
  }, [query, debounceMs])

  const searchSuggestionsKey = debouncedQuery
    ? (['google-search-suggestions', debouncedQuery] as const)
    : null

  return useSWR(searchSuggestionsKey, getSearchSuggestions, {
    keepPreviousData: true,
  })
}
