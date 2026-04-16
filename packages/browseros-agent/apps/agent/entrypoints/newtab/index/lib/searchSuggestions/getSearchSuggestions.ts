import type { SearchProviders } from './SearchProviders'

const getGoogleSuggestions = async (query: string): Promise<string[]> => {
  const response = await fetch(
    `https://suggestqueries.google.com/complete/search?client=chrome&q=${encodeURIComponent(query)}`,
  )
  const data = await response.json()
  return data[1] || []
}

const getBingSuggestions = async (query: string): Promise<string[]> => {
  const response = await fetch(
    `https://api.bing.com/osjson.aspx?query=${encodeURIComponent(query)}`,
  )
  const data = await response.json()
  return data[1] || []
}

interface YahooSuggestionItem {
  key: string
}

const getYahooIndiaSuggestions = async (query: string): Promise<string[]> => {
  const response = await fetch(
    `https://in.search.yahoo.com/sugg/gossip/gossip-in-loc/?command=${encodeURIComponent(query)}&output=json`,
  )
  const data = await response.json()
  return data?.gossip?.results?.map((item: YahooSuggestionItem) => item.key) ?? []
}

const getDuckDuckGoSuggestions = async (query: string): Promise<string[]> => {
  const response = await fetch(
    `https://duckduckgo.com/ac/?q=${encodeURIComponent(query)}&type=list`,
  )
  const data = await response.json()
  return data[1] || []
}

const getBraveSuggestions = async (query: string): Promise<string[]> => {
  const response = await fetch(
    `https://search.brave.com/api/suggest?q=${encodeURIComponent(query)}`,
  )
  const data = await response.json()
  return data[1] || []
}

/**
 * TODO: Move search suggestions fetching to background script to avoid CORS issues
 */
export const getSearchSuggestions = async ([searchEngine, query]: [
  searchEngine: SearchProviders,
  query: string,
]): Promise<string[]> => {
  switch (searchEngine) {
    case 'google':
      return getGoogleSuggestions(query)
    case 'bing':
      return getBingSuggestions(query)
    case 'yahoo':
      return getYahooIndiaSuggestions(query)
    case 'duckduckgo':
      return getDuckDuckGoSuggestions(query)
    case 'brave':
      return getBraveSuggestions(query)
    default:
      return []
  }
}
