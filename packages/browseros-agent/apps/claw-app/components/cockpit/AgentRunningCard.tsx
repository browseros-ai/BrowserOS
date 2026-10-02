import type { HelpRequest } from '@browseros/claw-api'
import { ExternalLink, Hand, RefreshCw, Square } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import { formatToolTrail, siteOf } from '@/screens/cockpit/cockpit.helpers'
import { activityCardCaptionTones } from './activityCardTone'
import { MiniScreencast } from './MiniScreencast'
import { needsYouTone } from './needsYouTone'
import { TabCountChip } from './TabCountChip'

type BrowserTabRecord = LiveSessionCardRecord['browserTabs'][number]

function formatWait(requestedAt: number, now: number): string {
  const seconds = Math.max(0, Math.floor((now - requestedAt) / 1000))
  const minutes = Math.floor(seconds / 60)
  return `${minutes}:${String(seconds % 60).padStart(2, '0')}`
}

interface SessionRunningCardProps {
  session: LiveSessionCardRecord
  /** The owned tab shown on the card: the reader's pick, else the live target. */
  selectedBrowserTabId?: number
  /** The session's current live target tab, for the live-versus-last-frame marker. */
  liveBrowserTabId?: number
  /** True once the reader pins a tab instead of following the live target. */
  pinned?: boolean
  onSelectTab?: (browserTabId: number) => void
  onFollowLive?: () => void
  onWatch?: () => void
  onTakeOver?: () => void
  onStop: () => void
  isFocusPending?: boolean
  isCancelPending?: boolean
}

/**
 * One card per connected session in the Running now strip. The shown browser
 * tab's latest screenshot dominates the top; the caption carries parent session
 * identity, recent tools, and Watch / Stop actions. The tab chip switches which
 * owned tab the card previews. A pinned tab keeps its own screenshot while the
 * LIVE marker follows the agent's current target.
 *
 * The LIVE indicator uses light blue rather than the vivid brand accent, which
 * is near-invisible on the saturated blue caption block.
 */
export function AgentRunningCard({
  session,
  selectedBrowserTabId,
  liveBrowserTabId,
  pinned,
  onSelectTab,
  onFollowLive,
  onWatch,
  onTakeOver,
  onStop,
  isFocusPending,
  isCancelPending,
}: SessionRunningCardProps) {
  const shownTab =
    session.browserTabs.find(
      (tab) => tab.browserTabId === selectedBrowserTabId,
    ) ?? session.browserTabs[0]
  const active = session.state === 'active'
  const isLiveTab =
    shownTab != null && shownTab.browserTabId === liveBrowserTabId
  const showingLive = active && isLiveTab
  const trail = formatToolTrail(session.recentTools)
  const site = shownTab ? siteOf(shownTab.url) : 'No browser activity'
  const help = session.helpRequest
  const needsHelp = help != null

  if (needsHelp) {
    return (
      <AgentNeedsYouCard
        session={session}
        help={help}
        shownTab={shownTab}
        site={site}
        liveBrowserTabId={liveBrowserTabId}
        onSelectTab={onSelectTab}
        onTakeOver={onTakeOver}
        onStop={onStop}
        isFocusPending={isFocusPending}
        isCancelPending={isCancelPending}
      />
    )
  }

  return (
    <div
      data-session-card={session.sessionId}
      className="group relative flex h-[300px] flex-col overflow-hidden rounded-2xl border border-border-2 bg-bg-sunken transition-[border-color] duration-150 hover:border-accent/40"
    >
      <div className="relative flex-1 overflow-hidden">
        {/* rrweb players retained played events until card teardown, causing
            unbounded heap growth and reproduced renderer OOM crashes. Cards
            only need the latest image; keep rrweb on the dedicated Replay view. */}
        <MiniScreencast
          site={site}
          live={showingLive}
          sessionId={session.sessionId}
          browserTabId={shownTab?.browserTabId}
          className="h-full w-full"
        />
        {shownTab && onSelectTab && (
          <div className="absolute top-3 right-3">
            <TabCountChip
              browserTabs={session.browserTabs}
              selectedBrowserTabId={shownTab.browserTabId}
              liveBrowserTabId={liveBrowserTabId}
              onSelectTab={onSelectTab}
            />
          </div>
        )}
      </div>
      <div
        className={cn(
          'flex flex-col gap-1.5 px-4 py-3',
          activityCardCaptionTones.blue.surface,
        )}
        data-caption-tone="blue"
      >
        <div className="flex items-center gap-3 font-mono text-[10.5px] text-white/80 uppercase tracking-[0.08em]">
          <span className="inline-flex min-w-0 items-center gap-1.5">
            <span
              aria-hidden
              className="inline-block size-2 shrink-0 rounded-full"
              style={{ background: session.color }}
            />
            <span className="truncate text-white">{session.label}</span>
          </span>
          {showingLive ? (
            <span className="inline-flex shrink-0 items-center gap-1.5 text-[#8fb4ff]">
              <span
                aria-hidden
                className="inline-block size-1.5 animate-[pulse-dot_1.4s_ease-in-out_infinite] rounded-full bg-[#8fb4ff] shadow-[0_0_8px_hsl(221_100%_78%/0.6)]"
              />
              LIVE
            </span>
          ) : active && pinned && !isLiveTab ? (
            <span className="inline-flex shrink-0 items-center gap-2 text-white/45">
              Last frame
              {onFollowLive && (
                <button
                  type="button"
                  data-follow-live={session.sessionId}
                  onClick={onFollowLive}
                  className="text-[#8fb4ff] uppercase tracking-[0.08em] transition hover:text-white"
                >
                  Follow live
                </button>
              )}
            </span>
          ) : (
            <span className="shrink-0 text-white/45">Idle</span>
          )}
        </div>
        <h3 className="truncate font-semibold text-[14px] text-white leading-tight">
          {shownTab?.title || session.name || site}
        </h3>
        <p className="truncate font-mono text-[11px] text-white/60">
          {trail || (shownTab ? site : 'Waiting for browser activity')}
        </p>
        <div className="mt-1 flex items-center gap-2 border-white/10 border-t pt-2">
          {shownTab && onWatch && (
            <button
              type="button"
              data-watch-browser-tab={shownTab.browserTabId}
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

interface AgentNeedsYouCardProps {
  session: LiveSessionCardRecord
  help: HelpRequest
  shownTab?: BrowserTabRecord
  site: string
  liveBrowserTabId?: number
  onSelectTab?: (browserTabId: number) => void
  onTakeOver?: () => void
  onStop: () => void
  isFocusPending?: boolean
  isCancelPending?: boolean
}

/**
 * The needs-you state of a running card: a blocked agent that has asked for a
 * human. The live preview shows the actual blocking page (a login or captcha),
 * the caption carries the reason and a waiting timer, and the primary action
 * foregrounds the agent's tab so the human can act. Resolving happens from the
 * in-control bar, not here, so Take over only brings the page forward.
 */
function AgentNeedsYouCard({
  session,
  help,
  shownTab,
  site,
  liveBrowserTabId,
  onSelectTab,
  onTakeOver,
  onStop,
  isFocusPending,
  isCancelPending,
}: AgentNeedsYouCardProps) {
  const waiting = formatWait(help.requestedAt, Date.now())
  const context = help.url ? siteOf(help.url) : site

  return (
    <div
      data-session-card={session.sessionId}
      data-needs-you={session.sessionId}
      className="group relative flex h-[300px] flex-col overflow-hidden rounded-2xl border-2 border-[#b85c10] bg-bg-sunken shadow-[0_0_0_1px_rgba(184,92,16,0.25)]"
    >
      <div className="relative flex-1 overflow-hidden">
        <MiniScreencast
          site={context}
          live
          sessionId={session.sessionId}
          browserTabId={shownTab?.browserTabId}
          className="h-full w-full"
        />
        <div className="absolute top-3 left-3">
          <span className="inline-flex items-center rounded-md bg-[#b85c10] px-2 py-1 font-mono text-[10px] text-white uppercase tracking-[0.1em] shadow-sm">
            Paused here
          </span>
        </div>
        {shownTab && onSelectTab && (
          <div className="absolute top-3 right-3">
            <TabCountChip
              browserTabs={session.browserTabs}
              selectedBrowserTabId={shownTab.browserTabId}
              liveBrowserTabId={liveBrowserTabId}
              onSelectTab={onSelectTab}
            />
          </div>
        )}
      </div>
      <div
        className={cn('flex flex-col gap-1.5 px-4 py-3', needsYouTone.surface)}
        data-caption-tone="needs-you"
      >
        <div className="flex items-center gap-3 font-mono text-[10.5px] text-white/85 uppercase tracking-[0.08em]">
          <span className="inline-flex min-w-0 items-center gap-1.5">
            <span
              aria-hidden
              className="inline-block size-2 shrink-0 rounded-full"
              style={{ background: session.color }}
            />
            <span className="truncate text-white">{session.label}</span>
          </span>
          <span className="inline-flex shrink-0 items-center gap-1.5 text-white">
            Needs you
            <span className="tabular-nums">{waiting}</span>
          </span>
        </div>
        <h3 className="truncate font-semibold text-[14px] text-white leading-tight">
          {help.reason}
        </h3>
        <p className="truncate font-mono text-[11px] text-white/70">
          {help.details || context}
        </p>
        <div className="mt-1 flex items-center gap-2 border-white/15 border-t pt-2">
          <button
            type="button"
            data-take-over={session.sessionId}
            onClick={onTakeOver}
            disabled={isFocusPending}
            className="inline-flex flex-1 items-center justify-center gap-1.5 rounded-md bg-white px-2 py-1.5 font-mono text-[#b85c10] text-[10.5px] uppercase tracking-[0.08em] transition hover:bg-white/90 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {isFocusPending ? (
              <RefreshCw className="size-3 animate-spin" />
            ) : (
              <Hand className="size-3" />
            )}
            Take over
          </button>
          <button
            type="button"
            data-stop-session={session.sessionId}
            onClick={onStop}
            disabled={isCancelPending}
            aria-label={isCancelPending ? 'Cancelling session' : 'Stop session'}
            className="inline-flex min-w-[92px] flex-1 items-center justify-center gap-1.5 rounded-md bg-white/15 px-2 py-1.5 font-mono text-[10.5px] text-white uppercase tracking-[0.08em] transition hover:bg-red-500/40 disabled:cursor-not-allowed disabled:opacity-60"
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
