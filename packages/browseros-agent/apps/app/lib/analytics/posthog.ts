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
}

export { posthog }
