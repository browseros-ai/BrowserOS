import { useEffect, useState } from 'react'
import { useAcpxForOpenClawStorage } from './feature-flag-storage'

/**
 * Reactive accessor for the `feature.useAcpxForOpenClaw` flag. Hydrates
 * from extension storage on mount and refreshes whenever the value
 * changes (including from another extension surface flipping it).
 */
export function useAcpxForOpenClaw(): boolean {
  const [enabled, setEnabled] = useState(false)

  useEffect(() => {
    let cancelled = false
    void useAcpxForOpenClawStorage.getValue().then((value) => {
      if (!cancelled) setEnabled(value)
    })
    const unwatch = useAcpxForOpenClawStorage.watch((value) => {
      setEnabled(value)
    })
    return () => {
      cancelled = true
      unwatch()
    }
  }, [])

  return enabled
}
