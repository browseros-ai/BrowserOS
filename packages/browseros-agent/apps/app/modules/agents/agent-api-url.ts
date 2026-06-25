export function buildAgentApiUrl(baseUrl: string, path: string): string {
  const normalizedPath = path === '/' ? '' : path
  return `${baseUrl}/agents${normalizedPath}`
}
