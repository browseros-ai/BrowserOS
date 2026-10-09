import { POSTHOG_HOST, POSTHOG_KEY } from '@/modules/analytics/posthog-config'
import type { UltrafastPrice } from './ultrafast-price'

/**
 * Submitting the form explicitly requests waitlist registration. Send only
 * that submission, without initialising the analytics SDK or changing consent.
 * The email stays on the event; it never creates a PostHog person profile.
 */
export async function submitUltrafastSignup(input: {
  email: string
  price: UltrafastPrice
  distinctId: string
}): Promise<void> {
  if (!POSTHOG_KEY) throw new Error('Waitlist signup is unavailable')

  const response = await fetch(`${POSTHOG_HOST.replace(/\/+$/, '')}/i/v0/e/`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    credentials: 'omit',
    referrerPolicy: 'no-referrer',
    signal: AbortSignal.timeout(10_000),
    body: JSON.stringify({
      api_key: POSTHOG_KEY,
      event: 'ultrafast_waitlist_joined',
      distinct_id: input.distinctId,
      properties: {
        email: input.email,
        price_usd: input.price,
        $process_person_profile: false,
      },
    }),
  })
  if (!response.ok) throw new Error('Waitlist signup failed')
  const result: unknown = await response.json()
  if (
    !result ||
    typeof result !== 'object' ||
    !('status' in result) ||
    result.status !== 1
  ) {
    throw new Error('Waitlist signup was not accepted')
  }
}
