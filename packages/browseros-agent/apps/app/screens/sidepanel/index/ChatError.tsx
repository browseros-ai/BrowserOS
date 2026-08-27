import {
  type ChatError as ChatErrorEnvelope,
  parseChatErrorEnvelope,
} from '@browseros/shared/schemas/chat-error'
import { AlertCircle, RefreshCw } from 'lucide-react'
import type { FC } from 'react'
import { useMemo, useState } from 'react'
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

export interface ChatErrorProps {
  error: Error
  onRetry?: () => void
  providerType?: string
}

interface ChatErrorView {
  title: string
  text: string
  url?: string
  linkLabel?: string
  details?: string
  canRetry: boolean
  showSurvey: boolean
}

/**
 * Copy for codes the extension knows about. The server also sends a title and
 * message, used as-is for codes shipped after this build.
 */
const CODE_COPY: Partial<
  Record<ChatErrorEnvelope['code'], { title: string; linkLabel: string }>
> = {
  credits_exhausted: {
    title: 'Daily limit reached',
    linkLabel: 'View Usage & Billing',
  },
  rate_limited: { title: 'Rate limited', linkLabel: 'About daily limits' },
  auth_failed: {
    title: 'Authentication failed',
    linkLabel: 'Open AI settings',
  },
  provider_config: {
    title: 'Provider not configured',
    linkLabel: 'Open AI settings',
  },
  connection_failed: {
    title: 'Connection failed',
    linkLabel: 'View troubleshooting guide',
  },
  context_length: { title: 'Conversation too long', linkLabel: 'Learn more' },
  content_filter: { title: 'Content was rejected', linkLabel: 'Learn more' },
  provider_unavailable: {
    title: 'Provider unavailable',
    linkLabel: 'Learn more',
  },
}

const AI_SETTINGS_URL = '/app.html#/settings/ai'

function fromEnvelope(envelope: ChatErrorEnvelope): ChatErrorView {
  const known = CODE_COPY[envelope.code]
  const url =
    envelope.docsUrl ??
    (envelope.code === 'auth_failed' || envelope.code === 'provider_config'
      ? AI_SETTINGS_URL
      : undefined)

  return {
    title: known?.title ?? envelope.title,
    text: envelope.message,
    url,
    linkLabel: known?.linkLabel,
    details: envelope.details,
    canRetry: envelope.retryable,
    // The survey belongs to the BrowserOS daily-limit prompt ("add your own
    // key"), not to plain credit exhaustion, which links to billing instead.
    showSurvey:
      envelope.code === 'rate_limited' && envelope.provider === 'browseros',
  }
}

/**
 * Fallback for bare strings: client-side fetch failures, an older agent server,
 * or an error raised outside the classifier.
 */
function fromMessage(message: string, providerType?: string): ChatErrorView {
  const isBrowserosProvider = providerType === 'browseros'

  // All chat requests go through the local BrowserOS agent server, so any
  // fetch failure is always a local connection issue.
  if (message.includes('Failed to fetch') || message.includes('fetch failed')) {
    return {
      title: 'Connection failed',
      text: 'Unable to connect to BrowserOS agent. Follow below instructions.',
      url: 'https://docs.browseros.com/troubleshooting/connection-issues',
      linkLabel: 'View troubleshooting guide',
      canRetry: true,
      showSurvey: false,
    }
  }

  if (
    isBrowserosProvider &&
    (message.includes('CREDITS_EXHAUSTED') ||
      message.includes('Credits exhausted') ||
      message.includes('Daily credits exhausted'))
  ) {
    return {
      title: 'Daily limit reached',
      text: 'Daily credits exhausted. Credits reset at midnight UTC.',
      url: '/app.html#/settings/usage',
      linkLabel: 'View Usage & Billing',
      canRetry: false,
      showSurvey: false,
    }
  }

  if (
    isBrowserosProvider &&
    message.includes('BrowserOS LLM daily limit reached')
  ) {
    return {
      title: 'Daily limit reached',
      text: 'Add your own API key for unlimited usage.',
      url: 'https://dub.sh/browseros-usage-limit',
      linkLabel: 'About daily limits',
      canRetry: false,
      showSurvey: true,
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

  return {
    title: 'Something went wrong',
    text: text || 'An unexpected error occurred',
    url,
    canRetry: true,
    showSurvey: false,
  }
}

function buildView(message: string, providerType?: string): ChatErrorView {
  const envelope = parseChatErrorEnvelope(message)
  return envelope ? fromEnvelope(envelope) : fromMessage(message, providerType)
}

/** Pretty-print JSON details; leave already-formatted or non-JSON text as-is. */
function formatErrorDetails(details: string): string {
  try {
    return JSON.stringify(JSON.parse(details), null, 2)
  } catch {
    return details
  }
}

export const ChatError: FC<ChatErrorProps> = ({
  error,
  onRetry,
  providerType,
}) => {
  const [copiedDetails, setCopiedDetails] = useState(false)
  const view = buildView(error.message, providerType)

  const surveyUrl = useMemo(
    () =>
      `/app.html?page=survey&maxTurns=20&experimentId=daily_limit_${pickRandomDirection()}#/settings/survey`,
    [],
  )

  const canRetry = !!onRetry && view.canRetry

  const detailsText = view.details ? formatErrorDetails(view.details) : ''

  // Copy targets the full string, not the DOM, so the whole error is copied even
  // though the block clips it to a scroll area on screen.
  const copyDetails = async () => {
    if (!detailsText) return
    try {
      await navigator.clipboard.writeText(detailsText)
      setCopiedDetails(true)
      window.setTimeout(() => setCopiedDetails(false), 1500)
    } catch {
      setCopiedDetails(false)
    }
  }

  return (
    <div className="mx-4 flex flex-col items-center justify-center gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4">
      <div className="flex items-center gap-2 text-muted-foreground">
        <AlertCircle className="h-5 w-5" />
        <span className="font-medium text-sm">{view.title}</span>
      </div>
      <p className="text-center text-destructive text-xs">{view.text}</p>
      {view.url && view.linkLabel && !view.showSurvey && (
        <a
          href={view.url}
          target="_blank"
          rel="noopener noreferrer"
          className="text-muted-foreground text-xs underline hover:text-foreground"
        >
          {view.linkLabel}
        </a>
      )}
      {view.showSurvey && (
        <p className="text-muted-foreground text-xs">
          {view.url && (
            <>
              <a
                href={view.url}
                target="_blank"
                rel="noopener noreferrer"
                className="underline hover:text-foreground"
              >
                {view.linkLabel ?? 'About daily limits'}
              </a>
              {' or '}
            </>
          )}
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
      {detailsText && detailsText !== view.text && (
        <div className="w-full">
          <div className="mb-1 flex items-center justify-between px-0.5">
            <span className="text-[10px] text-muted-foreground uppercase tracking-wide">
              Full error
            </span>
            <button
              type="button"
              onClick={copyDetails}
              className="text-[10px] text-muted-foreground underline hover:text-foreground"
            >
              {copiedDetails ? 'Copied' : 'Copy'}
            </button>
          </div>
          <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-words rounded border border-border bg-muted p-2 text-left text-[10px] text-muted-foreground">
            {detailsText}
          </pre>
        </div>
      )}
      {canRetry && (
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
