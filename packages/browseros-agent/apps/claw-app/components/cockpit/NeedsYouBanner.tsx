import type { HelpRequest } from '@browseros/claw-api'
import { Hand, Square, TriangleAlert } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import { siteOf } from '@/screens/cockpit/cockpit.helpers'
import { needsYouTone } from './needsYouTone'

interface NeedsYouBannerProps {
  /** Every live session; the banner picks out the ones waiting on a human. */
  sessions: LiveSessionCardRecord[]
  onTakeOver: (sessionId: string, browserTabId: number) => void
  onStop: (sessionId: string) => void
  pendingTakeOverSessionId?: string
  cancelPendingSessionId?: string
}

function waitLabel(requestedAt: number, now: number): string {
  const seconds = Math.max(0, Math.floor((now - requestedAt) / 1000))
  const minutes = Math.floor(seconds / 60)
  return `${minutes}:${String(seconds % 60).padStart(2, '0')}`
}

/**
 * The prominent cockpit alert shown whenever at least one agent is parked
 * waiting for a human. It features the oldest request in full and, when several
 * agents need help at once, lists the rest so none is buried below the fold.
 */
export function NeedsYouBanner({
  sessions,
  onTakeOver,
  onStop,
  pendingTakeOverSessionId,
  cancelPendingSessionId,
}: NeedsYouBannerProps) {
  const pending = sessions
    .flatMap((session) =>
      session.helpRequest ? [{ session, help: session.helpRequest }] : [],
    )
    .sort((left, right) => left.help.requestedAt - right.help.requestedAt)

  if (pending.length === 0) return null

  const now = Date.now()
  const [featured, ...rest] = pending

  return (
    <section
      data-needs-you-banner
      aria-live="polite"
      className="rounded-2xl border border-amber/40 bg-amber-tint px-5 py-4"
    >
      <div className="flex items-start gap-4">
        <span className="mt-0.5 inline-flex size-9 shrink-0 items-center justify-center rounded-full bg-[#b85c10] text-white">
          <TriangleAlert className="size-4.5" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-3">
            <h2 className="font-semibold text-[15px] text-ink leading-tight">
              {pending.length === 1
                ? 'One agent needs you right now'
                : `${pending.length} agents need you right now`}
            </h2>
            {pending.length > 1 && (
              <span className="inline-flex shrink-0 items-center rounded-full bg-[#b85c10] px-2 py-0.5 font-mono text-[10px] text-white uppercase tracking-[0.1em]">
                {pending.length} need you
              </span>
            )}
          </div>
          <FeaturedRequest
            session={featured.session}
            help={featured.help}
            now={now}
            onTakeOver={onTakeOver}
            onStop={onStop}
            isTakeOverPending={
              pendingTakeOverSessionId === featured.session.sessionId
            }
            isStopPending={
              cancelPendingSessionId === featured.session.sessionId
            }
          />
        </div>
      </div>

      {rest.length > 0 && (
        <ul className="mt-3 flex flex-col gap-2 border-amber/30 border-t pt-3">
          {rest.map(({ session, help }) => (
            <li
              key={session.sessionId}
              className="flex items-center gap-3 text-[13px]"
            >
              <span
                aria-hidden
                className="inline-block size-2 shrink-0 rounded-full"
                style={{ background: session.color }}
              />
              <span className="min-w-0 flex-1 truncate text-ink">
                <span className="font-medium">{session.label}</span>
                <span className="text-ink-3"> · {help.reason}</span>
              </span>
              <span className="shrink-0 font-mono text-[11px] text-ink-3 tabular-nums">
                {waitLabel(help.requestedAt, now)}
              </span>
              <Button
                type="button"
                size="xs"
                data-take-over={session.sessionId}
                onClick={() => onTakeOver(session.sessionId, help.browserTabId)}
                disabled={pendingTakeOverSessionId === session.sessionId}
                className={cn('shrink-0', needsYouTone.action)}
              >
                Take over
              </Button>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

interface FeaturedRequestProps {
  session: LiveSessionCardRecord
  help: HelpRequest
  now: number
  onTakeOver: (sessionId: string, browserTabId: number) => void
  onStop: (sessionId: string) => void
  isTakeOverPending: boolean
  isStopPending: boolean
}

function FeaturedRequest({
  session,
  help,
  now,
  onTakeOver,
  onStop,
  isTakeOverPending,
  isStopPending,
}: FeaturedRequestProps) {
  const site = help.url ? siteOf(help.url) : undefined
  const context = [session.label, session.name, site]
    .filter(Boolean)
    .join(' · ')

  return (
    <>
      <p className="mt-1 font-mono text-[11px] text-ink-3 uppercase tracking-[0.06em]">
        {context}
      </p>
      <p className="mt-1 font-medium text-[14px] text-ink">{help.reason}</p>
      {help.resumeHint && (
        <p className="mt-0.5 text-[13px] text-ink-2 italic">
          “{help.resumeHint}”
        </p>
      )}
      <div className="mt-2.5 flex flex-wrap items-center gap-2">
        <span className="mr-auto inline-flex items-center gap-1.5 font-mono text-[11px] text-ink-3 uppercase tracking-[0.08em]">
          Waiting
          <span className="tabular-nums">
            {waitLabel(help.requestedAt, now)}
          </span>
        </span>
        <Button
          type="button"
          size="sm"
          data-take-over={session.sessionId}
          onClick={() => onTakeOver(session.sessionId, help.browserTabId)}
          disabled={isTakeOverPending}
          className={needsYouTone.action}
        >
          <Hand className="size-3.5" />
          Take over
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          data-stop-session={session.sessionId}
          onClick={() => onStop(session.sessionId)}
          disabled={isStopPending}
          aria-label="Stop session"
        >
          <Square className="size-3.5" />
          Stop
        </Button>
      </div>
    </>
  )
}
