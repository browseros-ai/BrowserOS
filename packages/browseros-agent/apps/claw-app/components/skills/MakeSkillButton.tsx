/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { SessionSummary } from '@browseros/claw-api'
import {
  ArrowRight,
  BookmarkPlus,
  Info,
  LoaderCircle,
  PenLine,
  RotateCcw,
} from 'lucide-react'
import { type ReactNode, useEffect, useState } from 'react'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/motion/tooltip'
import { FreeMakeSkillDialog } from '@/components/skills/FreeMakeSkillDialog'
import { MakeSkillConfirm } from '@/components/skills/MakeSkillConfirm'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { usePlan } from '@/modules/api/plan.hooks'
import { taskDayAndTime } from '@/screens/audit/audit.helpers'
import { SkillDraftSheet } from '@/screens/skill-draft/SkillDraftSheet'
import {
  canDraftSkills,
  canShowMakeSkill,
  draftProblem,
  isBusy,
  type MakeSkillState,
  type SkillCtaSurface,
  siteName,
} from '@/screens/skill-draft/skill-draft.helpers'
import { useMakeSkill } from '@/screens/skill-draft/skill-draft.hooks'

const LABELS: Record<MakeSkillState, { icon: ReactNode; text: string }> = {
  idle: { icon: <BookmarkPlus aria-hidden />, text: 'Make it a skill' },
  writing: {
    icon: <LoaderCircle aria-hidden className="motion-safe:animate-spin" />,
    text: 'Writing your skill…',
  },
  review: { icon: <PenLine aria-hidden />, text: 'Checking your skill' },
  draft: { icon: <PenLine aria-hidden />, text: 'Finish your skill' },
  saving: {
    icon: <LoaderCircle aria-hidden className="motion-safe:animate-spin" />,
    text: 'Saving…',
  },
  saved: { icon: <ArrowRight aria-hidden />, text: 'Open skill' },
  error: { icon: <RotateCcw aria-hidden />, text: 'Try again' },
  limited: { icon: <Info aria-hidden />, text: 'Make it a skill' },
}

const ORDER = Object.keys(LABELS) as MakeSkillState[]

const STATE_CLASS: Record<MakeSkillState, string> = {
  idle: 'bg-accent-tint text-accent-ink',
  writing: 'bg-accent-tint text-accent-ink opacity-85',
  review: 'cursor-default bg-accent-tint text-accent-ink opacity-85',
  draft: 'bg-accent-tint text-accent-ink',
  saving: 'cursor-default bg-accent-tint text-accent-ink opacity-85',
  saved: 'border-border bg-card text-primary',
  error: 'bg-amber-tint text-amber-ink',
  limited: 'cursor-help bg-bg-sunken text-ink-3',
}

interface MakeSkillButtonProps {
  task: SessionSummary
  /** `row` on Home and History, `page` in the task page header. */
  surface: 'row' | 'page'
  /** Which list a row is in, for analytics; the task page header is always `task_page`. */
  place?: 'home' | 'history'
  /** Rows other than the newest show the button only on hover or focus, from `sm` up. */
  revealOnHover?: boolean
  className?: string
}

/**
 * "Make it a skill" for one finished task, on every plan where ultrafast is on sale. With
 * ultrafast a click asks first, then neo writes a draft; on a free plan it shows how to have the
 * agent save it instead, so no paid draft is ever requested there. Home, History and the task page
 * all read one shared draft state, so the label is the same everywhere. The button keeps its
 * width (every label shares one grid cell); while a draft is read or saved it ignores clicks, and
 * while one is written a click reopens the sheet.
 */
export function MakeSkillButton({
  task,
  surface,
  place = 'history',
  revealOnHover = false,
  className,
}: MakeSkillButtonProps) {
  const ctaSurface: SkillCtaSurface = surface === 'page' ? 'task_page' : place
  const plan = usePlan()
  const entitled = canDraftSkills(plan.data?.features)
  // Where ultrafast is not on sale, the free path's "Get ultrafast" would lead nowhere.
  const onSale = entitled || plan.data?.available === true
  const available = onSale && canShowMakeSkill(task)
  const planName = entitled ? 'ultrafast' : 'free'
  const flow = useMakeSkill({
    sessionId: task.sessionId,
    skillName: task.skillName,
    enabled: entitled && available,
  })
  const [asking, setAsking] = useState(false)
  const [freeOpen, setFreeOpen] = useState(false)
  const offered = available && flow.state === 'idle'
  // Counting an impression is a side effect of the button appearing; no handler runs then.
  useEffect(() => {
    if (offered) countCtaShown(ctaSurface, planName)
  }, [offered, ctaSurface, planName])
  if (!available) return null

  const state = flow.state
  const busy = isBusy(state)
  const click = () => {
    if (state !== 'idle') {
      flow.start()
      return
    }
    track(AnalyticsEvent.SkillFromTaskClicked, {
      surface: ctaSurface,
      plan: planName,
    })
    if (entitled) {
      setAsking(true)
    } else {
      track(AnalyticsEvent.FreeMakeSkillDialogShown, { surface: ctaSurface })
      setFreeOpen(true)
    }
  }
  const button = (
    <button
      type="button"
      aria-disabled={busy || undefined}
      aria-live="polite"
      onClick={(event) => {
        // Rows sit inside a link to the task page; the button must not follow it.
        event.preventDefault()
        event.stopPropagation()
        click()
      }}
      className={cn(
        'relative inline-grid shrink-0 items-center justify-items-center whitespace-nowrap rounded-[9px] border border-transparent font-semibold transition-colors focus-visible:outline-2 focus-visible:outline-primary focus-visible:outline-offset-2 active:translate-y-px [&_svg]:size-[15px]',
        surface === 'page'
          ? 'h-[34px] px-3.5 text-[14px]'
          : 'h-[30px] px-3 text-[13.5px]',
        STATE_CLASS[state],
        revealOnHover &&
          state === 'idle' &&
          'sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100',
        className,
      )}
    >
      {ORDER.map((key) => (
        <span
          key={key}
          aria-hidden={key !== state}
          className={cn(
            'inline-flex items-center gap-[7px] [grid-area:1/1]',
            key !== state && 'invisible',
          )}
        >
          {LABELS[key].icon}
          {LABELS[key].text}
        </span>
      ))}
    </button>
  )

  return (
    <>
      {state === 'limited' ? (
        <Tooltip>
          <TooltipTrigger asChild>{button}</TooltipTrigger>
          <TooltipContent className="max-w-[300px]">
            {draftProblem(flow.status?.failure).title}{' '}
            {draftProblem(flow.status?.failure).detail} Your agent can still
            save one for you.
          </TooltipContent>
        </Tooltip>
      ) : (
        button
      )}
      {entitled ? (
        <MakeSkillConfirm
          task={task}
          open={asking}
          onOpenChange={(open) => {
            if (!open && asking) {
              track(AnalyticsEvent.SkillMakeDismissed, { surface: ctaSurface })
            }
            setAsking(open)
          }}
          onConfirm={() => {
            track(AnalyticsEvent.SkillMakeConfirmed, { surface: ctaSurface })
            setAsking(false)
            flow.start()
          }}
        />
      ) : (
        <FreeMakeSkillDialog
          task={task}
          surface={ctaSurface}
          open={freeOpen}
          onOpenChange={setFreeOpen}
        />
      )}
      <SkillDraftSheet
        open={flow.sheetOpen}
        startedAt={flow.startedAt}
        status={flow.status}
        source={sourceLine(task)}
        site={task.site}
        saving={state === 'saving'}
        onClose={flow.close}
        onRetry={flow.retry}
        onSave={flow.save}
      />
    </>
  )
}

/** Surfaces whose button has been counted as shown in this app load: one impression each. */
const countedSurfaces = new Set<SkillCtaSurface>()

function countCtaShown(surface: SkillCtaSurface, plan: string) {
  if (countedSurfaces.has(surface)) return
  countedSurfaces.add(surface)
  track(AnalyticsEvent.SkillFromTaskCtaShown, { surface, plan })
}

/** "From your Flipkart task, today at 18:39". */
function sourceLine(task: SessionSummary): string {
  const when = taskDayAndTime(task.startedAt, Date.now())
  // "today at 18:39" reads as part of the sentence; a date keeps its capital.
  const lower = /^(Today|Yesterday)\b/.test(when)
    ? when.charAt(0).toLowerCase() + when.slice(1)
    : when
  const name = siteName(task.site)
  return name ? `From your ${name} task, ${lower}` : `From your task, ${lower}`
}
