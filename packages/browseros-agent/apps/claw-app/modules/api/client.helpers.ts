export type ApiBaseUrlSources = {
  query: string | null | undefined
  stored: string | null | undefined
  launcher: string | null | undefined
  fallback: string
}

export const API_URL_STORAGE_KEY = 'browseros.claw-app.apiUrl'

/** Accepts numeric loopback only; `localhost` can go through DNS. */
export function isLoopbackCockpitUrl(
  value: string | null | undefined,
): value is string {
  if (!value) return false
  try {
    const url = new URL(value)
    return (
      url.protocol === 'http:' &&
      url.hostname === '127.0.0.1' &&
      url.port !== '' &&
      url.pathname === '/cockpit' &&
      url.search === '' &&
      url.hash === ''
    )
  } catch {
    return false
  }
}

/** Resolves the cockpit API URL from trusted local dev sources. */
export function resolveApiBaseUrlFromSources(
  sources: ApiBaseUrlSources,
): string {
  if (isLoopbackCockpitUrl(sources.query)) return sources.query
  if (isLoopbackCockpitUrl(sources.stored)) return sources.stored
  if (isLoopbackCockpitUrl(sources.launcher)) return sources.launcher
  return sources.fallback
}
