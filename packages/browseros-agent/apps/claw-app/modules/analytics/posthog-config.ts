/** Public ingestion credentials embedded by the extension release pipeline. */
export const POSTHOG_KEY = (
  import.meta.env.VITE_CLAW_POSTHOG_KEY as string | undefined
)?.trim()

// Optional release secrets may be empty. Never resolve ingestion requests
// against the extension origin when no host was configured.
export const POSTHOG_HOST =
  (import.meta.env.VITE_CLAW_POSTHOG_HOST as string | undefined)?.trim() ||
  'https://us.i.posthog.com'
