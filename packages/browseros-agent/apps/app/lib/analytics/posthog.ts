import posthog from 'posthog-js'
import 'posthog-js/dist/posthog-recorder'
import { env } from '../env'

// Session replay mirrors the whole DOM continuously. The side panel is a
// long-lived, unvirtualized streaming chat that never reloads, so the
// recorder's node mirror grows until the renderer OOMs (issue #1972).
// Disable replay there; keep it for the shorter-lived full-page contexts.
const isSidePanel =
  typeof window !== 'undefined' &&
  window.location.pathname.includes('sidepanel')

// Set once, the first time this build runs in a profile, so the reset below
// happens exactly once. Held in localStorage rather than extension storage
// because that is where PostHog keeps the identity being cleared, and because
// reading it has to be synchronous: init captures a pageview immediately.
const IDENTITY_RETIRED_KEY = 'browseros.analytics_identity_retired'

/**
 * Clears an identity left behind by sign-in.
 *
 * Signing in called posthog.identify with the account's user id, and signing
 * out called reset. Sign-in is gone and so is the reset, but the id it stored
 * is not: persistence is localStorage, so it survives the update and every
 * later event stays attributed to an account that no longer exists. Events are
 * machine-scoped now, and this is what makes that true of a profile that had
 * signed in rather than only of a fresh one.
 *
 * Once, not on every load. reset() mints a new anonymous id, so repeating it
 * would split one person into a new one on every page load.
 */
function retireSignedInIdentity() {
  try {
    if (localStorage.getItem(IDENTITY_RETIRED_KEY)) return
    posthog.reset()
    localStorage.setItem(IDENTITY_RETIRED_KEY, '1')
  } catch {
    // Storage can throw when site data is blocked. Analytics keeping a stale
    // id is better than analytics not loading.
  }
}

if (env.VITE_PUBLIC_POSTHOG_KEY && env.VITE_PUBLIC_POSTHOG_HOST) {
  posthog.init(env.VITE_PUBLIC_POSTHOG_KEY, {
    api_host: env.VITE_PUBLIC_POSTHOG_HOST,
    person_profiles: 'identified_only',
    disable_external_dependency_loading: true,
    disable_session_recording: isSidePanel,
    capture_pageview: true,
    autocapture: true,
    session_recording: {
      maskAllInputs: true,
    },
    persistence: 'localStorage',
    loaded: (posthog) => {
      posthog.register({
        extension_version: chrome.runtime.getManifest().version,
        ui_context: window.location.pathname,
      })
    },
  })
  retireSignedInIdentity()
}

export { posthog }
