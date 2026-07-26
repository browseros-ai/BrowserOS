export type ApiBaseUrlSources = {
  query: string | null | undefined
  stored: string | null | undefined
  launcher: string | null | undefined
  fallback: string
}

export const API_URL_STORAGE_KEY = 'browseros.claw-onboard.apiUrl'

/** Normalizes trusted root API URLs; `localhost` can go through DNS. */
export function normalizeLoopbackApiRootUrl(
  value: string | null | undefined,
): string | null {
  if (!value) return null
  try {
    const url = new URL(value)
    const isValid =
      url.protocol === 'http:' &&
      url.hostname === '127.0.0.1' &&
      url.port !== '' &&
      url.pathname === '/' &&
      url.search === '' &&
      url.hash === ''
    return isValid ? url.origin : null
  } catch {
    return null
  }
}

/** Resolves the Claw API URL from trusted local launch sources. */
export function resolveApiBaseUrlFromSources(
  sources: ApiBaseUrlSources,
): string {
  const query = normalizeLoopbackApiRootUrl(sources.query)
  if (query) return query
  const stored = normalizeLoopbackApiRootUrl(sources.stored)
  if (stored) return stored
  const launcher = normalizeLoopbackApiRootUrl(sources.launcher)
  if (launcher) return launcher
  return sources.fallback
}
