export const DEFAULT_BROWSEROS_API_URL = 'https://api.browseros.com'

/** Resolves and validates the BrowserOS API base URL for runtime and build config. */
export function parseBrowserOSApiUrl(value: string | undefined): string {
  const rawUrl = value?.trim() || DEFAULT_BROWSEROS_API_URL
  let url: URL

  try {
    url = new URL(rawUrl)
  } catch {
    throw new Error(
      'VITE_PUBLIC_BROWSEROS_API must be a valid URL including http:// or https://',
    )
  }

  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    throw new Error('VITE_PUBLIC_BROWSEROS_API must use http:// or https://')
  }

  return url.toString().replace(/\/$/, '')
}
