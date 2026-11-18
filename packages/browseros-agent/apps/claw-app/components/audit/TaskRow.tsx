/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Globe } from 'lucide-react'
import { NavLink, useLocation } from 'react-router'
import { SiteChip } from '@/components/favicon/SiteChip'
import { AgentMark } from '@/components/harness/AgentMark'
import { MakeSkillButton } from '@/components/skills/MakeSkillButton'
import { cn } from '@/lib/utils'
import {
  type TaskSummary,
  taskScreenshotUrl,
  useTaskScreenshotBaseUrl,
} from '@/modules/api/audit.hooks'
import {
  taskTime,
  taskWhen,
  tookLabel,
  ultrafastNote,
} from '@/screens/audit/audit.helpers'
import { SessionStatusBadge } from './SessionStatusBadge'
import { UltrafastNote } from './UltrafastMarks'

interface TaskRowProps {
  task: TaskSummary
  now: number
  /** Under a day heading the day is already said, so show only the time. */
  timeOnly?: boolean
  /**
   * Whether "Make it a skill" shows all the time. Only the newest finished task in a list does;
   * the rest show it on hover and focus.
   */
  skillAlways?: boolean
  /** Which list the row is in, for analytics on "Make it a skill". */
  skillPlace?: 'home' | 'history'
}

/**
 * One finished or running task: picture, name, summary, agent, when and status. The whole row
 * links to the task page through the title's stretched link, so the skill button beside the
 * status can be a real button rather than one nested inside a link.
 */
export function TaskRow({
  task,
  now,
  timeOnly = false,
  skillAlways = false,
  skillPlace = 'history',
}: TaskRowProps) {
  const location = useLocation()
  const screenshotBaseUrl = useTaskScreenshotBaseUrl()
  const note = ultrafastNote(task.pursue, task.ultrafast)
  const shot =
    task.latestScreenshotId != null && screenshotBaseUrl !== null
      ? taskScreenshotUrl(
          task.sessionId,
          task.latestScreenshotId,
          screenshotBaseUrl,
        )
      : null

  return (
    <div
      data-task-row={task.sessionId}
      className="group relative grid grid-cols-[72px_minmax(0,1fr)] items-center gap-x-4 gap-y-2 px-4 py-3.5 transition-colors hover:bg-bg-canvas has-[a:focus-visible]:bg-bg-canvas sm:grid-cols-[112px_minmax(0,1fr)_auto] sm:gap-x-[18px] sm:px-[18px]"
    >
      <span
        className={cn(
          'relative grid aspect-[16/10] place-items-center overflow-hidden rounded-md border border-border-2',
          shot || !task.site ? 'bg-bg-sunken' : 'bg-accent-tint',
        )}
      >
        {shot ? (
          <>
            <img
              src={shot}
              alt=""
              loading="lazy"
              decoding="async"
              className="h-full w-full object-cover object-top-left"
            />
            {task.site ? (
              <SiteChip
                site={task.site}
                className="absolute bottom-1.5 left-1.5"
              />
            ) : null}
          </>
        ) : task.site ? (
          <SiteChip site={task.site} size="lg" />
        ) : (
          <Globe aria-hidden className="size-5 text-ink-3" />
        )}
      </span>
      <span className="flex min-w-0 flex-col gap-[3px]">
        <NavLink
          to={`/audit/${encodeURIComponent(task.sessionId)}`}
          state={{ from: location.pathname }}
          className="truncate font-semibold text-[15.5px] text-cyanotype-ink after:absolute after:inset-0 focus-visible:outline-none"
        >
          {task.name}
          {task.site ? <span className="sr-only">, on {task.site}</span> : null}
        </NavLink>
        {task.taskSummary ? (
          <span className="line-clamp-1 text-[14px] text-ink-2">
            {task.taskSummary}
          </span>
        ) : null}
        <span className="flex flex-wrap items-center gap-2 text-[13.5px] text-ink-3">
          <AgentMark
            harness={task.harness}
            label={task.label || task.slug}
            color={task.color}
          />
          <span aria-hidden>·</span>
          <span>
            {timeOnly
              ? taskTime(task.startedAt)
              : taskWhen(task.startedAt, now)}
          </span>
          <span aria-hidden>·</span>
          <span>{tookLabel(task.durationMs)}</span>
        </span>
        {note ? <UltrafastNote className="mt-0.5">{note}</UltrafastNote> : null}
      </span>
      <span className="relative col-start-2 flex w-full flex-row-reverse items-center justify-between gap-2.5 sm:col-start-auto sm:w-auto sm:flex-row sm:justify-end">
        <MakeSkillButton
          task={task}
          surface="row"
          place={skillPlace}
          revealOnHover={!skillAlways}
        />
        <SessionStatusBadge status={task.status} sessionId={task.sessionId} />
      </span>
    </div>
  )
}
