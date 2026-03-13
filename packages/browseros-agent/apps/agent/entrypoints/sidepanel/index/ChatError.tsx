import { AlertCircle, RefreshCw } from 'lucide-react'
import type { FC } from 'react'
// import { useMemo } from 'react'
import { Button } from '@/components/ui/button'
import {
  KIMI_RATE_LIMIT_DOCS_CLICKED_EVENT,
  KIMI_RATE_LIMIT_PLATFORM_CLICKED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'

// --- Commented out for Kimi partnership launch (restore after) ---
// const SURVEY_DIRECTIONS = [
//   'competitor',
//   'switching',
//   'workflow',
//   'activation',
// ] as const
//
// function pickRandomDirection(): string {
//   return SURVEY_DIRECTIONS[Math.floor(Math.random() * SURVEY_DIRECTIONS.length)]
// }
// --- End commented out survey code ---

interface ChatErrorProps {
  error: Error
  onRetry?: () => void
}

function parseErrorMessage(message: string): {
  text: string
  url?: string
  isRateLimit?: boolean
  isConnectionError?: boolean
} {
  // Detect MCP server connection failures
  if (
    (message.includes('Failed to fetch') || message.includes('fetch failed')) &&
    message.includes('127.0.0.1')
  ) {
    return {
      text: 'Unable to connect to BrowserOS agent. Follow below instructions.',
      url: 'https://docs.browseros.com/troubleshooting/connection-issues',
      isConnectionError: true,
    }
  }

  // Detect BrowserOS rate limit (unique pattern, no provider uses this)
  if (message.includes('BrowserOS LLM daily limit reached')) {
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

export const ChatError: FC<ChatErrorProps> = ({ error, onRetry }) => {
  const { text, url, isRateLimit, isConnectionError } = parseErrorMessage(
    error.message,
  )

  // --- Commented out for Kimi partnership launch (restore after) ---
  // const surveyUrl = useMemo(
  //   () =>
  //     `/app.html?page=survey&maxTurns=20&experimentId=daily_limit_${pickRandomDirection()}#/settings/survey`,
  //   [],
  // )
  // --- End commented out survey code ---

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
      {/* --- Commented out for Kimi partnership launch (restore after) ---
      {isRateLimit && (
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
      --- End commented out survey code --- */}
      {isRateLimit && (
        <div className="flex flex-col items-center gap-1">
          <p className="text-muted-foreground text-xs">
            {/* biome-ignore lint/a11y/useValidAnchor: link with click tracking */}
            <a
              href="https://docs.browseros.com/features/bring-your-own-llm#kimi-k2-5-%E2%80%94-in-partnership-with-moonshot-ai"
              target="_blank"
              rel="noopener noreferrer"
              className="underline hover:text-foreground"
              onClick={() => track(KIMI_RATE_LIMIT_DOCS_CLICKED_EVENT)}
            >
              Learn how to get a Kimi API key
            </a>
            {' or '}
            <a
              href="https://platform.moonshot.ai"
              target="_blank"
              rel="noopener noreferrer"
              className="underline hover:text-foreground"
              onClick={() => track(KIMI_RATE_LIMIT_PLATFORM_CLICKED_EVENT)}
            >
              get your API key
            </a>
          </p>
        </div>
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
