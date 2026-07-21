import { ExternalLink, RefreshCw, Square } from 'lucide-react'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import { formatToolTrail, siteOf } from '@/screens/cockpit/cockpit.helpers'
import { MiniScreencast } from './MiniScreencast'
import { TabCountChip } from './TabCountChip'

interface SessionRunningCardProps {
  session: LiveSessionCardRecord
  onWatch?: () => void
  onStop: () => void
  isFocusPending?: boolean
  isCancelPending?: boolean
}

/**
 * One card per connected session in the Running now strip. The selected
 * browser-tab preview dominates the top; the caption carries parent session
 * identity, recent tools, and Watch / Stop actions. Sessions without browser
 * tabs keep the same card shell so Stop remains available.
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
}: SessionRunningCardProps) {
  const selectedTab = session.selectedTab
  const active = session.state === 'active'
  const trail = formatToolTrail(session.recentTools)
  const site = selectedTab ? siteOf(selectedTab.url) : 'No browser activity'

  return (
    <div
      data-session-card={session.sessionId}
      className="group relative flex h-[300px] flex-col overflow-hidden rounded-2xl border border-border-2 bg-bg-sunken transition-[border-color] duration-150 hover:border-accent/40"
    >
      <div className="relative flex-1 overflow-hidden">
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
      <div className="flex flex-col gap-1.5 bg-ink-deep px-4 py-3 text-white">
        <div className="flex items-center gap-3 font-mono text-[10.5px] text-white/80 uppercase tracking-[0.08em]">
          <span className="inline-flex min-w-0 items-center gap-1.5">
            <span
              aria-hidden
              className="inline-block size-2 shrink-0 rounded-full"
              style={{ background: session.color }}
            />
            <span className="truncate text-white">{session.label}</span>
            <span className="shrink-0 text-white/45">{session.harness}</span>
          </span>
          <span
            className={
              active
                ? 'inline-flex shrink-0 items-center gap-1.5 text-[#8fb4ff]'
                : 'shrink-0 text-white/45'
            }
          >
            {active && (
              <span
                aria-hidden
                className="inline-block size-1.5 animate-[pulse-dot_1.4s_ease-in-out_infinite] rounded-full bg-[#8fb4ff] shadow-[0_0_8px_hsl(221_100%_78%/0.6)]"
              />
            )}
            {active ? 'LIVE' : 'Idle'}
          </span>
        </div>
        <h3 className="truncate font-semibold text-[14px] text-white leading-tight">
          {selectedTab?.title || session.name || site}
        </h3>
        <p className="truncate font-mono text-[11px] text-white/60">
          {trail || (selectedTab ? site : 'Waiting for browser activity')}
        </p>
        <div className="mt-1 flex items-center gap-2 border-white/10 border-t pt-2">
          {selectedTab && onWatch && (
            <button
              type="button"
              data-watch-browser-tab={selectedTab.browserTabId}
              onClick={onWatch}
              disabled={isFocusPending}
              className="inline-flex flex-1 items-center justify-center gap-1.5 rounded-md bg-white/10 px-2 py-1.5 font-mono text-[10.5px] text-white/90 uppercase tracking-[0.08em] transition hover:bg-white/20 disabled:cursor-not-allowed disabled:opacity-60"
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
            className="inline-flex min-w-[92px] flex-1 items-center justify-center gap-1.5 rounded-md bg-white/10 px-2 py-1.5 font-mono text-[10.5px] text-white/90 uppercase tracking-[0.08em] transition hover:bg-red-500/30 hover:text-red-100 disabled:cursor-not-allowed disabled:opacity-60"
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
