import { ExternalLink, RefreshCw, Square } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import { formatToolTrail, siteOf } from '@/screens/cockpit/cockpit.helpers'
import { formatElapsed } from '@/screens/cockpit/newtab.helpers'
import { MiniScreencast } from './MiniScreencast'
import { TabCountChip } from './TabCountChip'

interface SessionRunningCardProps {
  session: LiveSessionCardRecord
  onWatch?: () => void
  onStop: () => void
  isFocusPending?: boolean
  isCancelPending?: boolean
  /** `lead` renders the large hero card; `compact` renders the slim rail card. */
  variant?: 'lead' | 'compact'
  /** When set, the caption shows a live elapsed timer derived from `startedAt`. */
  now?: number
}

/**
 * One live session card. The selected browser-tab preview dominates the top;
 * the caption carries parent session identity, the recent tool trail, and
 * Watch / Stop actions. Sessions without browser tabs keep the same card
 * shell so Stop remains available.
 *
 * The LIVE indicator uses light blue rather than the vivid brand accent,
 * which is near-invisible on the dark caption block.
 */
export function AgentRunningCard({
  session,
  onWatch,
  onStop,
  isFocusPending,
  isCancelPending,
  variant = 'compact',
  now,
}: SessionRunningCardProps) {
  const selectedTab = session.selectedTab
  const active = session.state === 'active'
  const trail = formatToolTrail(session.recentTools)
  const site = selectedTab ? siteOf(selectedTab.url) : 'No browser activity'
  const isLead = variant === 'lead'
  const elapsed =
    now !== undefined ? formatElapsed(session.startedAt, now) : null

  return (
    <div
      data-session-card={session.sessionId}
      className={cn(
        'group relative flex flex-col overflow-hidden rounded-2xl border border-border-2 bg-bg-sunken transition-[border-color] duration-150 hover:border-accent/40',
        isLead ? 'h-full' : 'h-[236px]',
      )}
    >
      <div
        className={cn(
          'relative overflow-hidden',
          isLead ? 'aspect-video w-full' : 'flex-1',
        )}
      >
        <MiniScreencast
          site={site}
          live={active}
          sessionId={session.sessionId}
          className="h-full w-full"
        />
        {selectedTab && (
          <div className="absolute top-3 right-3">
            <TabCountChip
              browserTabs={session.browserTabs}
              selectedBrowserTabId={selectedTab.browserTabId}
            />
          </div>
        )}
      </div>
      <div
        className={cn(
          'flex flex-col gap-1.5 bg-ink-deep text-white',
          isLead ? 'flex-1 px-5 py-4' : 'px-4 py-3',
        )}
      >
        <div className="flex items-center gap-2 text-[11px] text-white/80">
          <span className="inline-flex min-w-0 items-center gap-1.5">
            <span
              aria-hidden
              className="inline-block size-2 shrink-0 rounded-full"
              style={{ background: session.color }}
            />
            <span className="truncate font-medium text-white">
              {session.label}
            </span>
            <span className="shrink-0 text-white/45">{session.harness}</span>
          </span>
          <span
            className={cn(
              'ml-auto shrink-0',
              active
                ? 'inline-flex items-center gap-1.5 text-[#8fb4ff]'
                : 'text-white/45',
            )}
          >
            {active && (
              <span
                aria-hidden
                className="inline-block size-1.5 animate-pulse-dot rounded-full bg-[#8fb4ff] shadow-[0_0_8px_hsl(221_100%_78%/0.6)]"
              />
            )}
            {active ? 'Live' : 'Idle'}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span
            className={cn(
              'min-w-0 flex-1 truncate font-semibold text-white leading-tight',
              isLead ? 'text-[15px]' : 'text-[13px]',
            )}
          >
            {selectedTab?.title || session.name || site}
          </span>
          {elapsed !== null && (
            <span className="shrink-0 font-mono text-[11px] text-white/60 tabular-nums">
              {elapsed}
            </span>
          )}
        </div>
        <p className="truncate font-mono text-[11px] text-white/55">
          {trail || (selectedTab ? site : 'Waiting for browser activity')}
        </p>
        <div className="mt-1 flex items-center gap-2 border-white/10 border-t pt-2">
          {selectedTab && onWatch && (
            <button
              type="button"
              data-watch-browser-tab={selectedTab.browserTabId}
              onClick={onWatch}
              disabled={isFocusPending}
              className="inline-flex flex-1 items-center justify-center gap-1.5 rounded-md bg-white/10 px-2 py-1.5 text-[12px] text-white/90 transition hover:bg-white/20 disabled:cursor-not-allowed disabled:opacity-60"
            >
              {isFocusPending ? (
                <RefreshCw className="size-3 animate-spin" />
              ) : (
                <ExternalLink className="size-3" />
              )}
              Watch
            </button>
          )}
          <button
            type="button"
            data-stop-session={session.sessionId}
            onClick={onStop}
            disabled={isCancelPending}
            aria-label={isCancelPending ? 'Cancelling session' : 'Stop session'}
            // min-w reserves enough width for the longer pending label so
            // swapping states does not push the adjacent Watch button around.
            className="inline-flex min-w-[92px] flex-1 items-center justify-center gap-1.5 rounded-md bg-white/10 px-2 py-1.5 text-[12px] text-white/90 transition hover:bg-red-500/30 hover:text-red-100 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {isCancelPending ? (
              <>
                <RefreshCw className="size-3 animate-spin" /> Cancelling
              </>
            ) : (
              <>
                <Square className="size-3" /> Stop
              </>
            )}
          </button>
        </div>
      </div>
    </div>
  )
}
