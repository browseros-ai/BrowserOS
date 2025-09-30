/**
 * Fetches Google search suggestions for a given query
 * @param query - The search query to get suggestions for
 * @returns Promise<string[]> - Array of suggestion strings
 */
export async function fetchGoogleSuggestions(query: string): Promise<string[]> {
  if (!query || query.length < 2) {
    return []
  }

  try {
    const url = `https://suggestqueries.google.com/complete/search?client=firefox&q=${encodeURIComponent(query)}`

    const response = await fetch(url, {
      method: 'GET',
      headers: {
        'Accept': 'application/json',
      },
    })

    if (!response.ok) {
      throw new Error(`HTTP error! status: ${response.status}`)
    }

    const data = await response.json() as [string, string[]]

    // Return the suggestions array (second element in the response)
    return data[1] || []
  } catch (error) {
    console.error('Failed to fetch Google suggestions:', error)
    return []
  }
}
