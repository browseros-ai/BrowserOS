import { useEffect, useState } from 'react'
import { importHintDismissedAtStorage } from '@/lib/onboarding/onboardingStorage'

const DISMISS_DURATION = 7 * 24 * 60 * 60 * 1000

function isEligible(dismissedAt: number | null): boolean {
  return !dismissedAt || Date.now() - dismissedAt >= DISMISS_DURATION
}

/** Delays the import prompt on home visits while honoring its saved dismissal. */
export function useShowImportHint(): boolean {
  const [show, setShow] = useState(false)

  useEffect(() => {
    let cancelled = false
    let timer: ReturnType<typeof setTimeout> | undefined

    async function resolve() {
      const importDismissedAt = await importHintDismissedAtStorage.getValue()
      if (cancelled) return

      if (isEligible(importDismissedAt)) {
        timer = setTimeout(() => {
          if (!cancelled) setShow(true)
        }, 2000)
      }
    }

    resolve()
    return () => {
      // Home can unmount before storage resolves or the display delay expires.
      cancelled = true
      clearTimeout(timer)
    }
  }, [])

  return show
}
