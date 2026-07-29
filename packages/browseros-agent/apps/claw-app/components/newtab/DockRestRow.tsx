import { ExternalLink, RefreshCw, Square } from 'lucide-react'
import { MiniScreencast } from '@/components/cockpit/MiniScreencast'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import { formatElapsed, siteOf } from '@/screens/cockpit/cockpit.helpers'

interface DockRestRowProps {
  session: LiveSessionCardRecord
  now: number
  onWatch?: () => void
  onStop: () => void
  isFocusPending?: boolean
  isCancelPending?: boolean
}

/** Compact live row for sessions past the lead: small thumb, meta, icon controls. */
export function DockRestRow({
  session,
  now,
  onWatch,
  onStop,
  isFocusPending,
  isCancelPending,
}: DockRestRowProps) {
  const selectedTab = session.selectedTab
  const active = session.state === 'active'
  const site = selectedTab ? siteOf(selectedTab.url) : 'No browser activity'

  return (
    <div
      className="flex items-center gap-3 rounded-xl border border-border-2 bg-card px-3 py-2"
      data-session-card={session.sessionId}
    >
      <div className="size-12 shrink-0 overflow-hidden rounded-lg">
        <MiniScreencast
          className="size-full"
          live={active}
          sessionId={session.sessionId}
          site={site}
        />
      </div>
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <div className="flex min-w-0 items-center gap-1.5">
          <span
            aria-hidden
            className="size-2 shrink-0 rounded-full"
            style={{ background: session.color }}
          />
          <span className="truncate font-medium text-ink text-sm">
            {session.label}
          </span>
          <span className="shrink-0 text-ink-3 text-xs">{session.harness}</span>
        </div>
        <div className="flex min-w-0 items-center gap-2 text-ink-3 text-xs">
          <span className="truncate">{site}</span>
          <span className="shrink-0 font-mono tabular-nums">
            {formatElapsed(now - session.startedAt)}
          </span>
          {active ? (
            <span className="inline-flex shrink-0 items-center gap-1 text-accent-ink">
              <span
                aria-hidden
                className="inline-block size-1 animate-pulse-dot rounded-full bg-accent"
              />
              LIVE
            </span>
          ) : null}
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-1">
        {selectedTab && onWatch ? (
          <button
            aria-label="Watch session"
            className="inline-flex size-8 items-center justify-center rounded-md border border-border-2 bg-card text-ink-2 transition hover:border-border-strong hover:text-ink disabled:cursor-not-allowed disabled:opacity-60"
            data-watch-browser-tab={selectedTab.browserTabId}
            disabled={isFocusPending}
            onClick={onWatch}
            type="button"
          >
            {isFocusPending ? (
              <RefreshCw className="size-3.5 animate-spin" />
            ) : (
              <ExternalLink className="size-3.5" />
            )}
          </button>
        ) : null}
        <button
          aria-label={isCancelPending ? 'Cancelling session' : 'Stop session'}
          className="inline-flex size-8 items-center justify-center rounded-md border border-border-2 bg-card text-ink-2 transition hover:border-red/40 hover:text-red disabled:cursor-not-allowed disabled:opacity-60"
          data-stop-session={session.sessionId}
          disabled={isCancelPending}
          onClick={onStop}
          type="button"
        >
          {isCancelPending ? (
            <RefreshCw className="size-3.5 animate-spin" />
          ) : (
            <Square className="size-3.5" />
          )}
        </button>
      </div>
    </div>
  )
}
