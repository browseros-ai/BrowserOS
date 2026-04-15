import { AlertCircle, RefreshCw } from 'lucide-react'
import type { FC } from 'react'
import { useMemo } from 'react'
import { ShareForCredits } from '@/components/referral/ShareForCredits'
import { Button } from '@/components/ui/button'

const SURVEY_DIRECTIONS = [
  'competitor',
  'switching',
  'workflow',
  'activation',
] as const

function pickRandomDirection(): string {
  return SURVEY_DIRECTIONS[Math.floor(Math.random() * SURVEY_DIRECTIONS.length)]
}

interface ChatErrorProps {
  error: Error
  onRetry?: () => void
  providerType?: string
}

function parseErrorMessage(
  message: string,
  providerType?: string,
): {
  text: string
  url?: string
  isRateLimit?: boolean
  isCreditsExhausted?: boolean
  isConnectionError?: boolean
} {
  const isBrowserosProvider = providerType === 'browseros'

  // All chat requests go through the local BrowserOS agent server, so any
  // fetch failure is always a local connection issue.
  if (message.includes('Failed to fetch') || message.includes('fetch failed')) {
    return {
      text: 'Unable to connect to BrowserOS agent. Follow below instructions.',
      url: 'https://docs.browseros.com/troubleshooting/connection-issues',
      isConnectionError: true,
    }
  }

  // Detect credit exhaustion from gateway (BrowserOS provider only)
  if (
    isBrowserosProvider &&
    (message.includes('CREDITS_EXHAUSTED') ||
      message.includes('Credits exhausted') ||
      message.includes('Daily credits exhausted'))
  ) {
    return {
      text: 'Daily credits exhausted. Credits reset at midnight UTC.',
      url: '/app.html#/settings/usage',
      isRateLimit: true,
      isCreditsExhausted: true,
    }
  }

  // Detect BrowserOS rate limit (BrowserOS provider only)
  if (
    isBrowserosProvider &&
    message.includes('BrowserOS LLM daily limit reached')
  ) {
    return {
      text: 'Add your own API key for unlimited usage.',
      url: 'https://dub.sh/browseros-usage-limit',
      isRateLimit: true,
    }
  }

  let text = message
  try {
    const parsed = JSON.parse(message)
    if (parsed?.error?.message) text = parsed.error.message
  } catch {}

  // Extract URL if present
  const urlMatch = text.match(/https?:\/\/[^\s]+/)
  const url = urlMatch?.[0]
  if (url) {
    text = text.replace(url, '').replace(/\s+/g, ' ').trim()
  }

  return { text: text || 'An unexpected error occurred', url }
}

export const ChatError: FC<ChatErrorProps> = ({
  error,
  onRetry,
  providerType,
}) => {
  const { text, url, isRateLimit, isCreditsExhausted, isConnectionError } =
    parseErrorMessage(error.message, providerType)

  const surveyUrl = useMemo(
    () =>
      `/app.html?page=survey&maxTurns=20&experimentId=daily_limit_${pickRandomDirection()}#/settings/survey`,
    [],
  )

  const getTitle = () => {
    if (isRateLimit) return 'Daily limit reached'
    if (isConnectionError) return 'Connection failed'
    return 'Something went wrong'
  }

  return (
    <div className="mx-4 flex flex-col items-center justify-center gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4">
      <div className="flex items-center gap-2 text-muted-foreground">
        <AlertCircle className="h-5 w-5" />
        <span className="font-medium text-sm">{getTitle()}</span>
      </div>
      <p className="text-center text-destructive text-xs">{text}</p>
      {isConnectionError && url && (
        <a
          href={url}
          target="_blank"
          rel="noopener noreferrer"
          className="text-muted-foreground text-xs underline hover:text-foreground"
        >
          View troubleshooting guide
        </a>
      )}
      {isCreditsExhausted && (
        <>
          <div className="w-full border-border/50 border-t pt-3">
            <ShareForCredits compact />
          </div>
          {url && (
            <a
              href={url}
              target="_blank"
              rel="noopener noreferrer"
              className="text-muted-foreground text-xs underline hover:text-foreground"
            >
              View Usage & Billing
            </a>
          )}
        </>
      )}
      {isRateLimit && !isCreditsExhausted && (
        <p className="text-muted-foreground text-xs">
          <a
            href={url}
            target="_blank"
            rel="noopener noreferrer"
            className="underline hover:text-foreground"
          >
            Learn more
          </a>
          {' or '}
          <a
            href={surveyUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="underline hover:text-foreground"
          >
            take a quick survey
          </a>
        </p>
      )}
      {onRetry && (
        <Button
          variant="outline"
          size="sm"
          onClick={onRetry}
          className="mt-1 gap-2"
        >
          <RefreshCw className="h-3.5 w-3.5" />
          Try again
        </Button>
      )}
    </div>
  )
}
