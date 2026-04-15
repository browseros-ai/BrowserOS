const REFERRAL_SERVICE_URL = 'https://browseros-referral.fly.dev'

interface ReferralResult {
  success: boolean
  creditsAdded?: number
  reason?: string
}

export async function submitReferral(
  tweetUrl: string,
  browserosId: string,
): Promise<ReferralResult> {
  const response = await fetch(`${REFERRAL_SERVICE_URL}/referral/submit`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ tweetUrl, browserosId }),
  })
  return response.json()
}

export function getShareOnTwitterUrl(): string {
  // TODO: Rotate between 20-30 variations
  const text = 'I use @browseros_ai to browse the web with AI. Check it out!'
  return `https://x.com/intent/tweet?text=${encodeURIComponent(text)}`
}
