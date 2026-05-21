/**
 * Fetches new-tab search suggestions.
 * 
 * Security Hardening: Disabled search suggestions to prevent data leakage to external search engines.
 */
export const getSearchSuggestions = async (): Promise<string[]> => {
  return []
}
