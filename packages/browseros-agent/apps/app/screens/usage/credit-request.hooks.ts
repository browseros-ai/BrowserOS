import { useMutation } from '@tanstack/react-query'
import { BROWSEROS_GATEWAY_URL } from '@/lib/constants/gatewayUrl'
import { sentry } from '@/lib/sentry/sentry'

export const DISCORD_HANDLE_MAX_LENGTH = 64

export const CREDIT_REQUEST_RATE_LIMITED_MESSAGE =
  "You've sent a few requests already — please try again later."

export const CREDIT_REQUEST_FAILED_MESSAGE =
  'Something went wrong, please try again.'

export interface CreditRequestInput {
  browserosId: string
  discordHandle: string
}

/** Throws with a message that is safe to render as-is. */
export async function submitCreditRequest(
  input: CreditRequestInput,
): Promise<void> {
  let response: Response
  try {
    response = await fetch(`${BROWSEROS_GATEWAY_URL}/credit-request`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(input),
    })
  } catch (error) {
    sentry.captureException(error, {
      extra: { message: 'Failed to reach the gateway for a credit request' },
    })
    throw new Error(CREDIT_REQUEST_FAILED_MESSAGE)
  }

  // Any ok status is an accept, including a duplicate the gateway suppressed.
  if (response.ok) return

  throw new Error(
    response.status === 429
      ? CREDIT_REQUEST_RATE_LIMITED_MESSAGE
      : CREDIT_REQUEST_FAILED_MESSAGE,
  )
}

export function useSubmitCreditRequest() {
  return useMutation({ mutationFn: submitCreditRequest })
}
