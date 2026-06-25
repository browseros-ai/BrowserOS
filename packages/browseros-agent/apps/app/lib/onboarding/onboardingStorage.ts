import { storage } from '@wxt-dev/storage'

interface OnboardingProfile {
  name: string
  role: string
  company: string
  description?: string
}

export const onboardingCompletedStorage = storage.defineItem<boolean>(
  'local:onboardingCompleted',
  { fallback: false },
)

export const onboardingProfileStorage =
  storage.defineItem<OnboardingProfile | null>('local:onboardingProfile', {
    fallback: null,
  })

export const importHintDismissedAtStorage = storage.defineItem<number | null>(
  'local:importHintDismissedAt',
  { fallback: null },
)

export const signInHintDismissedAtStorage = storage.defineItem<number | null>(
  'local:signInHintDismissedAt',
  { fallback: null },
)

export const authRedirectPathStorage = storage.defineItem<string | null>(
  'local:authRedirectPath',
  { fallback: null },
)

export const firstRunConfettiShownStorage = storage.defineItem<boolean>(
  'local:firstRunConfettiShown',
  { fallback: false },
)
