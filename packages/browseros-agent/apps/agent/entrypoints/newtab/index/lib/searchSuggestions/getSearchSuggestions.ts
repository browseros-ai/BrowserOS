import type { SearchProviders } from './SearchProviders'

export const getSearchSuggestions = async ([searchEngine, query]: [
  SearchProviders,
  string,
]): Promise<string[]> => {
  // Security Hardening: Disabled search suggestions to prevent data leakage to 5 different search engines
  return []
}
