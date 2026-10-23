import type { HelpRequest } from '@browseros/claw-api'
import { CircleCheck, Hand, KeyRound, ShieldCheck } from 'lucide-react'
import {
  Alert,
  AlertAction,
  AlertContent,
  AlertDescription,
  AlertTitle,
} from '@/components/motion/alert'
import { Button } from '@/components/motion/button/base'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import {
  helpRequestSite,
  helpRequestTitle,
  waitingLabel,
} from '@/screens/cockpit/cockpit.helpers'

interface NeedsYouBannerProps {
  /** Every live session; the banner picks out the ones waiting on a human. */
  sessions: LiveSessionCardRecord[]
  onTakeOver: (sessionId: string, browserTabId: number) => void
  onStop: (sessionId: string) => void
  pendingTakeOverSessionId?: string
  cancelPendingSessionId?: string
}

const KIND_ICON = {
  login: KeyRound,
  captcha: ShieldCheck,
  approval: CircleCheck,
  other: Hand,
} as const

const KIND_HINT = {
  login: 'Sign in there, then come back here.',
  captcha: 'Finish the check there, then come back here.',
  approval: 'Look it over there, then come back here.',
  other: 'Do what it asks there, then come back here.',
} as const

/**
 * Every agent waiting on a person, oldest first, each as its own request in
 * plain words with one clear action. This is the only amber surface on Home.
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

  return (
    <section
      data-needs-you-banner
      aria-label="Waiting for you"
      className="ph-no-capture flex flex-col gap-3"
    >
      {pending.map(({ session, help }) => (
        <HelpRequestAlert
          key={session.sessionId}
          session={session}
          help={help}
          now={now}
          onTakeOver={onTakeOver}
          onStop={onStop}
          isTakeOverPending={pendingTakeOverSessionId === session.sessionId}
          isStopPending={cancelPendingSessionId === session.sessionId}
        />
      ))}
    </section>
  )
}

// ---------------------------------------------------------------------------

interface HelpRequestAlertProps {
  session: LiveSessionCardRecord
  help: HelpRequest
  now: number
  onTakeOver: (sessionId: string, browserTabId: number) => void
  onStop: (sessionId: string) => void
  isTakeOverPending: boolean
  isStopPending: boolean
}

function HelpRequestAlert({
  session,
  help,
  now,
  onTakeOver,
  onStop,
  isTakeOverPending,
  isStopPending,
}: HelpRequestAlertProps) {
  const kind = help.kind ?? 'other'
  const Icon = KIND_ICON[kind]
  const site = helpRequestSite(help)

  return (
    <Alert
      variant="warning"
      className="rounded-2xl px-5 py-5 md:px-6"
      contentClassName="gap-4 md:gap-[18px]"
    >
      <span
        aria-hidden
        className="grid size-11 shrink-0 place-items-center rounded-full border border-amber/40 bg-card text-amber-ink"
      >
        <Icon className="size-[22px]" strokeWidth={1.75} />
      </span>
      <AlertContent className="flex min-w-0 flex-col gap-2.5 space-y-0">
        <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
          <AlertTitle className="font-bold text-[19px] text-cyanotype-ink leading-snug tracking-[-0.015em]">
            {helpRequestTitle(session.agentName, help)}
          </AlertTitle>
          <span className="whitespace-nowrap font-semibold text-[13.5px] text-amber-ink">
            {waitingLabel(help.requestedAt, now)}
          </span>
        </div>
        <AlertDescription className="max-w-[64ch] text-[15px] text-ink-2">
          “{help.reason}”
        </AlertDescription>
        <AlertAction className="gap-2.5 pt-0.5">
          <Button
            data-take-over={session.sessionId}
            onClick={() => onTakeOver(session.sessionId, help.browserTabId)}
            disabled={isTakeOverPending}
            className="font-semibold"
          >
            <Hand aria-hidden className="size-4" />
            Help now
          </Button>
          <Button
            variant="ghost"
            data-stop-session={session.sessionId}
            onClick={() => onStop(session.sessionId)}
            disabled={isStopPending}
            className="font-semibold"
          >
            Stop task
          </Button>
          <span className="text-[13.5px] text-ink-3">
            We’ll bring the {site} page to the front. {KIND_HINT[kind]}
          </span>
        </AlertAction>
      </AlertContent>
    </Alert>
  )
}
