import { Eye, Hand, Square } from 'lucide-react'
import { AgentProgress } from '@/components/agents/loading-states/agent-progress'
import { AgentMark } from '@/components/harness/AgentMark'
import { AnimatedBadge } from '@/components/motion/animated-badge'
import { AspectRatio } from '@/components/motion/aspect-ratio'
import { Button } from '@/components/motion/button/base'
import { Card } from '@/components/motion/card'
import { describeLiveStep } from '@/lib/step-sentences'
import { cn } from '@/lib/utils'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import { helpRequestSite, siteOf } from '@/screens/cockpit/cockpit.helpers'
import { MiniScreencast } from './MiniScreencast'

interface AgentRunningCardProps {
  session: LiveSessionCardRecord
  /** True while the person is in control of this session's page. */
  inControl?: boolean
  onWatch?: () => void
  onTakeOver?: () => void
  onStop: () => void
  isFocusPending?: boolean
  isCancelPending?: boolean
}

/**
 * One task an agent is doing right now: a framed live picture of the page,
 * what it is doing in a sentence, which agent, and how many pages it has open.
 */
export function AgentRunningCard({
  session,
  inControl,
  onWatch,
  onTakeOver,
  onStop,
  isFocusPending,
  isCancelPending,
}: AgentRunningCardProps) {
  // browserTabs is sorted by activity, so the first tab is the one in use.
  const tab = session.browserTabs[0]
  const help = session.helpRequest
  const waiting = help !== undefined
  const site = help
    ? helpRequestSite(help)
    : tab
      ? siteOf(tab.url)
      : 'No page yet'
  const pages = session.browserTabs.length
  const startedSeconds = Math.max(0, (Date.now() - session.startedAt) / 1000)

  return (
    <Card
      data-session-card={session.sessionId}
      data-needs-you={waiting ? session.sessionId : undefined}
      size="compact"
      className={cn(
        'gap-0 border-border-2 p-3 shadow-card',
        // Working cards take the same calm blue as Home's promotion bar;
        // waiting ones stay plain so amber alone says "needs you".
        waiting || inControl ? 'bg-card' : 'bg-accent-tint',
      )}
    >
      <AspectRatio
        ratio={16 / 9}
        animated={false}
        className="rounded-[10px] bg-bg-sunken shadow-shot"
      >
        <MiniScreencast
          site={site}
          live={session.state === 'active'}
          sessionId={session.sessionId}
          browserTabId={tab?.browserTabId}
          className="h-full w-full"
        />
        <AnimatedBadge
          status={waiting ? 'warning' : 'loading'}
          size="sm"
          className="absolute top-2 left-2 shadow-card"
        >
          {inControl
            ? 'You’re in control'
            : waiting
              ? 'Waiting for you'
              : 'Working'}
        </AnimatedBadge>
      </AspectRatio>
      <div className="flex flex-1 flex-col gap-1 px-1 pt-3 pb-0.5">
        <h3 className="font-bold text-[15.5px] text-cyanotype-ink leading-[1.3] tracking-[-0.01em]">
          {session.name || tab?.title || site}
        </h3>
        {waiting ? (
          <p className="text-[14px] text-ink-2">{help.reason}</p>
        ) : (
          <AgentProgress
            label={describeLiveStep(tab)}
            initialSeconds={startedSeconds}
            running={session.state === 'active'}
            className="min-w-0 gap-2 font-sans text-[14px] text-ink-2 [&>span:last-child]:whitespace-nowrap [&>span:nth-child(2)]:min-w-0 [&>span:nth-child(2)]:truncate"
          />
        )}
        <p className="flex flex-wrap items-center gap-2 text-[13.5px] text-ink-3">
          <AgentMark
            harness={session.harness}
            label={session.label}
            color={session.color}
          />
          <span aria-hidden>·</span>
          <span>
            {pages} {pages === 1 ? 'page' : 'pages'}
          </span>
        </p>
        <div className="mt-auto flex gap-2 pt-2.5">
          {waiting && !inControl && onTakeOver ? (
            <Button
              size="sm"
              data-take-over={session.sessionId}
              onClick={onTakeOver}
              disabled={isFocusPending}
              className="h-[34px] px-3.5 font-semibold text-[13.5px]"
            >
              <Hand aria-hidden className="size-[15px]" />
              Help now
            </Button>
          ) : tab && onWatch ? (
            <Button
              size="sm"
              variant="secondary"
              data-watch-browser-tab={tab.browserTabId}
              onClick={onWatch}
              disabled={isFocusPending}
              className="h-[34px] px-3.5 font-semibold text-[13.5px]"
            >
              <Eye aria-hidden className="size-[15px]" />
              {inControl ? 'Go to the page' : 'Watch'}
            </Button>
          ) : null}
          <Button
            size="sm"
            variant="ghost"
            data-stop-session={session.sessionId}
            onClick={onStop}
            disabled={isCancelPending}
            aria-label={isCancelPending ? 'Stopping the task' : 'Stop the task'}
            className="h-[34px] px-3 font-semibold text-[13.5px]"
          >
            <Square aria-hidden className="size-[15px]" />
            {isCancelPending ? 'Stopping' : 'Stop'}
          </Button>
        </div>
      </div>
    </Card>
  )
}
