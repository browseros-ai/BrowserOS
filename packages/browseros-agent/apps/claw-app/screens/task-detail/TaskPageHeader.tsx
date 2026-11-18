import type { SessionSummary } from '@browseros/claw-api'
import { ArrowRight, BookmarkPlus, ExternalLink } from 'lucide-react'
import { NavLink, useLocation } from 'react-router'
import { SessionStatusBadge } from '@/components/audit/SessionStatusBadge'
import { UltrafastWordmark } from '@/components/audit/UltrafastMarks'
import { AgentMark } from '@/components/harness/AgentMark'
import { PageHeader } from '@/components/layout/PageHeader'
import { ButtonLink } from '@/components/motion/button/base'
import { MakeSkillButton } from '@/components/skills/MakeSkillButton'
import { usePlan } from '@/modules/api/plan.hooks'
import { taskDayAndTime, tookLabel } from '@/screens/audit/audit.helpers'
import {
  isDraftable,
  ULTRAFAST_FEATURE,
} from '@/screens/skill-draft/skill-draft.helpers'
import { UltrafastLine } from './UltrafastLine'

interface TaskPageHeaderProps {
  session: SessionSummary
  /** The last web page the task was on, if any. */
  lastPageUrl?: string
}

/**
 * Status, agent, when and how long, then the task in a sentence and its summary, and on an
 * ultrafast task what neo did.
 */
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
    <div className="flex flex-col gap-3">
      <PageHeader
        back={back}
        eyebrow={
          <>
            <SessionStatusBadge
              status={session.status}
              sessionId={session.sessionId}
              size="md"
            />
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
        title={
          <span className="text-[26px] md:text-[28px]">{session.name}</span>
        }
        lede={
          session.taskSummary ? (
            <p className="max-w-[68ch] text-[17px] text-ink leading-[1.55]">
              {session.taskSummary}
            </p>
          ) : undefined
        }
        actions={
          <>
            <MakeSkillButton task={session} surface="page" />
            {lastPageUrl ? (
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
            ) : null}
          </>
        }
      />
      <UltrafastLine session={session} />
      <SkillUpsell session={session} />
    </div>
  )
}

/**
 * One quiet line on the task page for a plan without ultrafast: Home and History rows stay as
 * they are for these plans.
 */
function SkillUpsell({ session }: { session: SessionSummary }) {
  const plan = usePlan()
  // Only where ultrafast is on sale, so the link never leads to a plan that cannot be bought.
  if (
    !plan.data?.available ||
    plan.data.features.includes(ULTRAFAST_FEATURE) ||
    !isDraftable(session)
  ) {
    return null
  }
  return (
    <p className="flex flex-wrap items-center gap-x-3 gap-y-2 text-[14.5px] text-ink-2">
      <BookmarkPlus aria-hidden className="size-4" />
      <span>
        With <UltrafastWordmark /> you can turn this task into a skill in one
        click.
      </span>
      <NavLink
        to="/pro"
        className="inline-flex items-center gap-1 font-semibold text-primary"
      >
        See ultrafast
        <ArrowRight aria-hidden className="size-[15px]" />
      </NavLink>
    </p>
  )
}
