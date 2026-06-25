import { useEffect, useState } from 'react'
import { useSessionInfo } from '@/lib/auth/sessionStorage'
import {
  importHintDismissedAtStorage,
  signInHintDismissedAtStorage,
} from '@/lib/onboarding/onboardingStorage'

export type HintType = 'import' | 'signin'

const DISMISS_DURATION = 7 * 24 * 60 * 60 * 1000

function isEligible(dismissedAt: number | null): boolean {
  return !dismissedAt || Date.now() - dismissedAt >= DISMISS_DURATION
}

export function useActiveHint(): HintType | null {
  const [hint, setHint] = useState<HintType | null>(null)
  const { sessionInfo, isLoading } = useSessionInfo()

  useEffect(() => {
    if (isLoading) return

    let cancelled = false
    let timer: ReturnType<typeof setTimeout>

    async function resolve() {
      const importDismissedAt = await importHintDismissedAtStorage.getValue()
      if (cancelled) return

      if (isEligible(importDismissedAt)) {
        timer = setTimeout(() => {
          if (!cancelled) setHint('import')
        }, 2000)
        return
      }

      if (sessionInfo?.user) return

      const signinDismissedAt = await signInHintDismissedAtStorage.getValue()
      if (cancelled) return

      if (isEligible(signinDismissedAt)) {
        timer = setTimeout(() => {
          if (!cancelled) setHint('signin')
        }, 2000)
      }
    }

    resolve()
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [isLoading, sessionInfo?.user])

  return hint
}
