import { useQueryClient } from '@tanstack/react-query'
import { MessageSquareText } from 'lucide-react'
import { CopyButton } from '@/components/copy/CopyButton'
import { Card } from '@/components/motion/card'
import { useLiveSessions, useSessions } from '@/modules/api/audit.hooks'
import { useCancelSession } from '@/modules/api/cancel.hooks'
import { useFocusBrowserTab } from '@/modules/api/focus.hooks'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import { TRY_PROMPT } from '@/screens/cockpit/cockpit-onboarding.helpers'
import { AgentRunningCard } from './AgentRunningCard'

interface RunningGridProps {
  sessions: LiveSessionCardRecord[]
  /** Foreground a blocked agent's tab and open the in-control bar. */
  onTakeOver?: (sessionId: string, browserTabId: number) => void
  takeOverPendingSessionId?: string
  /** The session whose page the person is in control of, if any. */
  inControlSessionId?: string
}

/** What the agents are doing now, one compact card per task, or a calm idle line. */
export function RunningGrid({
  sessions,
  onTakeOver,
  takeOverPendingSessionId,
  inControlSessionId,
}: RunningGridProps) {
  const queryClient = useQueryClient()
  const focus = useFocusBrowserTab()
  const cancel = useCancelSession()

  if (sessions.length === 0) return <IdleCard />

  const onWatch = (browserTabId: number) => {
    focus.mutate(
      { browserTabId },
      {
        onError: (err) => {
          // eslint-disable-next-line no-console
          console.warn('focus browser tab failed', { browserTabId, err })
        },
      },
    )
  }
  const onStop = (sessionId: string) => {
    cancel.mutate(
      { sessionId },
      {
        onSuccess: () => {
          void queryClient.invalidateQueries({
            queryKey: useLiveSessions.getKey(),
          })
          void queryClient.invalidateQueries({
            queryKey: useSessions.getKey(),
          })
        },
        onError: (err) => {
          // eslint-disable-next-line no-console
          console.warn('cancel session failed', { sessionId, err })
        },
      },
    )
  }
  const pendingBrowserTabId =
    focus.isPending && focus.variables
      ? focus.variables.browserTabId
      : undefined
  const cancelPendingSessionId =
    cancel.isPending && cancel.variables
      ? cancel.variables.sessionId
      : undefined

  return (
    <section
      aria-labelledby="happening-now"
      className="ph-no-capture flex flex-col gap-3.5"
    >
      <h2
        id="happening-now"
        className="font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]"
      >
        Happening now
      </h2>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(280px,1fr))] gap-4">
        {sessions.map((session) => {
          const tabId = session.browserTabs[0]?.browserTabId
          const help = session.helpRequest
          return (
            <AgentRunningCard
              key={session.sessionId}
              session={session}
              inControl={inControlSessionId === session.sessionId}
              onWatch={tabId !== undefined ? () => onWatch(tabId) : undefined}
              onTakeOver={
                help && onTakeOver
                  ? () => onTakeOver(session.sessionId, help.browserTabId)
                  : undefined
              }
              onStop={() => onStop(session.sessionId)}
              isFocusPending={
                help
                  ? takeOverPendingSessionId === session.sessionId
                  : pendingBrowserTabId === tabId
              }
              isCancelPending={cancelPendingSessionId === session.sessionId}
            />
          )
        })}
      </div>
    </section>
  )
}

// ---------------------------------------------------------------------------

function IdleCard() {
  return (
    <Card
      variant="muted"
      size="compact"
      className="flex-row flex-wrap items-center gap-4 px-5 py-5 md:flex-nowrap"
    >
      <span
        aria-hidden
        className="grid size-11 shrink-0 place-items-center rounded-full bg-accent-tint text-primary"
      >
        <MessageSquareText className="size-[18px]" strokeWidth={1.75} />
      </span>
      <div className="flex min-w-0 flex-1 basis-[calc(100%-60px)] flex-col gap-0.5 md:basis-auto">
        <p className="font-bold text-[16px] text-cyanotype-ink">
          Ask your AI agents to use neo
        </p>
        <p className="text-[14.5px] text-ink-2">
          Start your message with “use neo” in Claude or any connected agent.
          You’ll see it here while it works.
        </p>
      </div>
      <CopyButton
        text={TRY_PROMPT}
        label="Copy an example"
        size="sm"
        className="ml-[60px] h-[34px] font-semibold text-[13.5px] md:ml-0"
      />
    </Card>
  )
}
