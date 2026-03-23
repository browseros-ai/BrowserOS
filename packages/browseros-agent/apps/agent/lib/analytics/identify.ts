import { sentry } from '../sentry/sentry'
import { posthog } from './posthog'

/**
 * Identify the current user across all analytics and error tracking services.
 * Call this when the user logs in or when a stored session is restored.
 */
export function identify(user: { id: string; email?: string; name?: string }) {
  sentry.setUser({ id: user.id, email: user.email })
  posthog.identify(user.id, {
    email: user.email,
    name: user.name,
  })
}

/**
 * Clear user identity across all services.
 * Call this when the user logs out.
 */
export function resetIdentity() {
  sentry.setUser(null)
  posthog.reset()
}
