/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Globe } from 'lucide-react'
import { NavLink, useLocation } from 'react-router'
import { SiteChip } from '@/components/favicon/SiteChip'
import { AgentMark } from '@/components/harness/AgentMark'
import { cn } from '@/lib/utils'
import {
  type TaskSummary,
  taskScreenshotUrl,
  useTaskScreenshotBaseUrl,
} from '@/modules/api/audit.hooks'
import { taskTime, taskWhen, tookLabel } from '@/screens/audit/audit.helpers'
import { SessionStatusBadge } from './SessionStatusBadge'

interface TaskRowProps {
  task: TaskSummary
  now: number
  /** Under a day heading the day is already said, so show only the time. */
  timeOnly?: boolean
}

/** One finished or running task: picture, name, summary, agent, when and status. */
export function TaskRow({ task, now, timeOnly = false }: TaskRowProps) {
  const location = useLocation()
  const screenshotBaseUrl = useTaskScreenshotBaseUrl()
  const shot =
    task.latestScreenshotId != null && screenshotBaseUrl !== null
      ? taskScreenshotUrl(
          task.sessionId,
          task.latestScreenshotId,
          screenshotBaseUrl,
        )
      : null

  return (
    <NavLink
      to={`/audit/${encodeURIComponent(task.sessionId)}`}
      state={{ from: location.pathname }}
      data-task-row={task.sessionId}
      className="grid grid-cols-[72px_minmax(0,1fr)] items-center gap-x-4 gap-y-2 px-4 py-3.5 transition-colors hover:bg-bg-canvas focus-visible:bg-bg-canvas focus-visible:outline-none sm:grid-cols-[112px_minmax(0,1fr)_auto] sm:gap-x-[18px] sm:px-[18px]"
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
        <span className="truncate font-semibold text-[15.5px] text-cyanotype-ink">
          {task.name}
          {task.site ? <span className="sr-only">, on {task.site}</span> : null}
        </span>
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
      </span>
      <SessionStatusBadge
        status={task.status}
        className="col-start-2 justify-self-start sm:col-start-auto sm:justify-self-end"
      />
    </NavLink>
  )
}
