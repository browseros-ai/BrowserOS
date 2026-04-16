import { EXTERNAL_URLS } from '@browseros/shared/constants/urls'

interface ReferralResult {
  success: boolean
  creditsAdded?: number
  reason?: string
}

export async function submitReferral(
  tweetUrl: string,
  browserosId: string,
): Promise<ReferralResult> {
  const response = await fetch(
    `${EXTERNAL_URLS.REFERRAL_SERVICE}/referral/submit`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ tweetUrl, browserosId }),
    },
  )
  if (!response.ok) {
    return {
      success: false,
      reason: `Request failed with status ${response.status}`,
    }
  }
  return response.json()
}

export function getShareOnTwitterUrl(): string {
  const text = 'I use @browseros_ai to browse the web with AI. Check it out!'
  return `https://x.com/intent/tweet?text=${encodeURIComponent(text)}`
}
