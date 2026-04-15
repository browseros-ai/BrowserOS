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

  const handleSubmit = async () => {
    if (!tweetUrl.trim() || !data?.browserosId) return

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

  return (
    <div className={compact ? 'space-y-2' : 'space-y-3'}>
      <p className={compact ? 'text-muted-foreground text-xs' : 'text-sm'}>
        Share BrowserOS on Twitter to earn 200 bonus credits!
      </p>

      <Button variant="outline" size="sm" className="w-full gap-2" asChild>
        <a
          href={getShareOnTwitterUrl()}
          target="_blank"
          rel="noopener noreferrer"
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
