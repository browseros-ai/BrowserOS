import { useCallback, useEffect, useRef, useState } from 'react'

export const NEKO_URL = 'http://localhost:8080'
const POLL_INTERVAL_MS = 4000

export type NekoStatus = 'checking' | 'online' | 'offline'

/**
 * Polls the local neko server to determine if it is running.
 * Returns `online` when the server responds, `offline` when it doesn't,
 * and `checking` on the initial probe.
 */
export function useNekoStatus(): { status: NekoStatus; retry: () => void } {
  const [status, setStatus] = useState<NekoStatus>('checking')
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null)

  const probe = useCallback(async () => {
    try {
      // Use no-cors so the browser doesn't block the request cross-origin.
      // A successful fetch (even opaque) means the server is reachable.
      const res = await fetch(NEKO_URL, {
        method: 'HEAD',
        mode: 'no-cors',
        cache: 'no-store',
        signal: AbortSignal.timeout(2500),
      })
      // opaque responses have status 0 but the fetch itself succeeded
      if (res.type === 'opaque' || res.ok) {
        setStatus('online')
      } else {
        setStatus('offline')
      }
    } catch {
      setStatus('offline')
    }
  }, [])

  useEffect(() => {
    probe()
    intervalRef.current = setInterval(probe, POLL_INTERVAL_MS)
    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current)
    }
  }, [probe])

  const retry = useCallback(() => {
    setStatus('checking')
    probe()
  }, [probe])

  return { status, retry }
}
