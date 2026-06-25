import { useEffect, useState } from 'react'
import { getAgentServerUrl } from '../../lib/browseros/helpers'

const MAX_AGENT_SERVER_URL_ATTEMPTS = 3
const AGENT_SERVER_URL_RETRY_DELAY_MS = 500

export type UseAgentServerUrlResult =
  | { baseUrl: string; isLoading: false; error: null }
  | { baseUrl?: never; isLoading: true; error: null }
  | { baseUrl?: never; isLoading: false; error: Error }

/** Resolves the local BrowserOS server URL used by React surfaces. */
export function useAgentServerUrl(): UseAgentServerUrlResult {
  const [state, setState] = useState<UseAgentServerUrlResult>({
    isLoading: true,
    error: null,
  })

  useEffect(() => {
    let cancelled = false
    let retryTimer: ReturnType<typeof setTimeout> | undefined

    async function loadUrl(attempt: number) {
      try {
        const url = await getAgentServerUrl()
        if (!cancelled) {
          setState({ baseUrl: url, isLoading: false, error: null })
        }
      } catch (e) {
        if (!cancelled) {
          if (attempt < MAX_AGENT_SERVER_URL_ATTEMPTS) {
            retryTimer = setTimeout(() => {
              void loadUrl(attempt + 1)
            }, AGENT_SERVER_URL_RETRY_DELAY_MS)
            return
          }
          setState({
            isLoading: false,
            error: e instanceof Error ? e : new Error(String(e)),
          })
        }
      }
    }

    void loadUrl(1)

    return () => {
      cancelled = true
      if (retryTimer) {
        clearTimeout(retryTimer)
      }
    }
  }, [])

  return state
}
