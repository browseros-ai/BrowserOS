const getGoogleSuggestions = async (query: string): Promise<string[]> => {
  const response = await fetch(
    `https://suggestqueries.google.com/complete/search?client=chrome&q=${encodeURIComponent(query)}`,
  )
  const data = await response.json()
  return data[1] || []
}

type SearchSuggestionsKey = readonly ['google-search-suggestions', string]

/**
 * Fetches new-tab search suggestions from the fixed Google provider.
 *
 * TODO: Move search suggestions fetching to background script to avoid CORS issues
 */
export const getSearchSuggestions = async (
  key: SearchSuggestionsKey,
): Promise<string[]> => {
  const [, query] = key
  return getGoogleSuggestions(query)
}
