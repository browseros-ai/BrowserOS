import { env } from '@/lib/env'

/**
 * Origin of the BrowserOS LLM gateway. This is a different host from
 * `DEFAULT_BROWSEROS_API_URL` (api.browseros.com) — the two are not
 * interchangeable.
 *
 * @public
 */
export const BROWSEROS_GATEWAY_URL =
  env.VITE_PUBLIC_BROWSEROS_GATEWAY?.replace(/\/+$/, '') ??
  'https://llm.browseros.com'
