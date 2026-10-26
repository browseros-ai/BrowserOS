import type { SessionSummary } from '@browseros/claw-api'
import { ExternalLink } from 'lucide-react'
import { useLocation } from 'react-router'
import { SessionStatusBadge } from '@/components/audit/SessionStatusBadge'
import { AgentMark } from '@/components/harness/AgentMark'
import { PageHeader } from '@/components/layout/PageHeader'
import { ButtonLink } from '@/components/motion/button/base'
import { taskDayAndTime, tookLabel } from '@/screens/audit/audit.helpers'

interface TaskPageHeaderProps {
  session: SessionSummary
  /** The last web page the task was on, if any. */
  lastPageUrl?: string
}

/** Status, agent, when and how long, then the task in a sentence and its summary. */
export function TaskPageHeader({ session, lastPageUrl }: TaskPageHeaderProps) {
  const location = useLocation()
  // Router state carries where the person came from; a direct link goes to History.
  const from =
    typeof location.state === 'object' &&
    location.state !== null &&
    'from' in location.state &&
    typeof location.state.from === 'string'
      ? location.state.from
      : '/audit'
  const back =
    from === '/' ? { to: '/', label: 'Home' } : { to: from, label: 'History' }

  return (
    <PageHeader
      back={back}
      eyebrow={
        <>
          <SessionStatusBadge status={session.status} size="md" />
          <span className="flex flex-wrap items-center gap-2 text-[13.5px] text-ink-3">
            <AgentMark
              harness={session.harness}
              label={session.label || session.slug}
              color={session.color}
            />
            <span aria-hidden>·</span>
            <span>{taskDayAndTime(session.startedAt, Date.now())}</span>
            <span aria-hidden>·</span>
            <span>{tookLabel(session.durationMs)}</span>
          </span>
        </>
      }
      title={<span className="text-[26px] md:text-[28px]">{session.name}</span>}
      lede={
        session.taskSummary ? (
          <p className="max-w-[68ch] text-[17px] text-ink leading-[1.55]">
            {session.taskSummary}
          </p>
        ) : undefined
      }
      actions={
        lastPageUrl ? (
          <ButtonLink
            href={lastPageUrl}
            target="_blank"
            rel="noreferrer"
            variant="secondary"
            size="sm"
            className="h-[34px] px-3.5 font-semibold text-[13.5px]"
          >
            Open the last page
            <ExternalLink aria-hidden className="size-[15px]" />
          </ButtonLink>
        ) : undefined
      }
    />
  )
}
