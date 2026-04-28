import { REFERRAL_LIMITS } from '@browseros/shared/constants/limits'
import { ExternalLink, Loader2, Send } from 'lucide-react'
import type { FC } from 'react'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useCredits, useInvalidateCredits } from '@/lib/credits/useCredits'
import {
  getShareOnTwitterUrl,
  submitReferral,
} from '@/lib/referral/submit-referral'

interface ShareForCreditsProps {
  compact?: boolean
}

export const ShareForCredits: FC<ShareForCreditsProps> = ({ compact }) => {
  const [tweetUrl, setTweetUrl] = useState('')
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [result, setResult] = useState<{
    success: boolean
    message: string
  } | null>(null)

  const { data } = useCredits()
  const invalidateCredits = useInvalidateCredits()

  const credits = data?.credits ?? 0
  const atDailyMax = credits >= REFERRAL_LIMITS.MAX_DAILY_CREDITS

  const handleSubmit = async () => {
    if (!tweetUrl.trim() || !data?.browserosId || atDailyMax) return

    setIsSubmitting(true)
    setResult(null)

    try {
      const res = await submitReferral(tweetUrl.trim(), data.browserosId)
      if (res.success) {
        setResult({
          success: true,
          message: `${res.creditsAdded ?? 200} credits added!`,
        })
        setTweetUrl('')
        invalidateCredits()
      } else {
        setResult({
          success: false,
          message: res.reason ?? 'Submission failed. Please try again.',
        })
      }
    } catch {
      setResult({
        success: false,
        message: 'Network error. Please try again.',
      })
    } finally {
      setIsSubmitting(false)
    }
  }

  if (atDailyMax) {
    return (
      <div className={compact ? 'space-y-2' : 'space-y-3'}>
        <p className={compact ? 'text-muted-foreground text-xs' : 'text-sm'}>
          You've reached the daily cap of {REFERRAL_LIMITS.MAX_DAILY_CREDITS}{' '}
          credits. Come back tomorrow to earn more!
        </p>
      </div>
    )
  }

  return (
    <div className={compact ? 'space-y-2' : 'space-y-3'}>
      <p className={compact ? 'text-muted-foreground text-xs' : 'text-sm'}>
        Share BrowserOS on Twitter to earn{' '}
        {REFERRAL_LIMITS.CREDITS_PER_REFERRAL} bonus credits!
      </p>

      <ul className="list-disc space-y-0.5 pl-4 text-muted-foreground text-xs">
        <li>
          Tweet must mention <span className="font-medium">@browserOS_ai</span>
        </li>
        <li>Tweet must be posted within the last 30 minutes</li>
        <li>Each tweet can only be submitted once</li>
        <li>
          Daily cap of {REFERRAL_LIMITS.MAX_DAILY_CREDITS} credits — resets at
          midnight UTC
        </li>
      </ul>

      <Button variant="outline" size="sm" className="w-full gap-2" asChild>
        <a
          href={getShareOnTwitterUrl()}
          target="_blank"
          rel="noopener noreferrer"
          onClick={(e) => {
            e.currentTarget.href = getShareOnTwitterUrl()
          }}
        >
          <ExternalLink className="h-3.5 w-3.5" />
          Share on Twitter
        </a>
      </Button>

      <p className="text-muted-foreground text-xs">
        Already shared? Paste your tweet link:
      </p>

      <div className="flex gap-2">
        <Input
          type="url"
          placeholder="https://x.com/..."
          value={tweetUrl}
          onChange={(e) => setTweetUrl(e.target.value)}
          className="h-8 text-xs"
          disabled={isSubmitting}
        />
        <Button
          variant="default"
          size="sm"
          onClick={handleSubmit}
          disabled={isSubmitting || !tweetUrl.trim()}
          className="shrink-0 gap-1.5"
        >
          {isSubmitting ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Send className="h-3.5 w-3.5" />
          )}
          Submit
        </Button>
      </div>

      {result && (
        <p
          className={
            result.success
              ? 'text-green-600 text-xs dark:text-green-400'
              : 'text-destructive text-xs'
          }
        >
          {result.message}
        </p>
      )}
    </div>
  )
}
