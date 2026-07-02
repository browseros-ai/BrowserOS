import { ChevronRight } from 'lucide-react'
import { NavLink } from 'react-router'
import { AgentDot } from '@/components/audit/AgentDot'
import { StatusBadge } from '@/components/audit/StatusBadge'
import { AspectRatio } from '@/components/ui/aspect-ratio'
import { cn } from '@/lib/utils'
import {
  type TaskSummary,
  taskScreenshotUrl,
  useTaskScreenshotBaseUrl,
} from '@/modules/api/audit.hooks'
import {
  abbreviateSequence,
  formatDuration,
  formatRelative,
} from '@/screens/audit/audit.helpers'

interface TaskCardProps {
  task: TaskSummary
  now: number
}

export function TaskCard({ task, now }: TaskCardProps) {
  const screenshotBaseUrl = useTaskScreenshotBaseUrl()
  return (
    <NavLink
      to={`/audit/${encodeURIComponent(task.sessionId)}`}
      className={cn(
        // Restrict the transition to color + shadow so the inner image
        // is not re-rasterized on parent paints. The generic
        // Tailwind `transition` shorthand transitions transform too,
        // which causes the subpixel red-line rendering on the hero
        // screenshot to shift slightly during the hover window and
        // reads as "the image is animating".
        'group block rounded-2xl border border-border-2 bg-card p-4 transition-[border-color,box-shadow] duration-150 hover:border-accent/40 hover:shadow-sm',
        task.status === 'live' && 'border-accent/30',
      )}
      data-testid={`task-card-${task.sessionId}`}
    >
      <header className="flex items-center gap-2">
        <AgentDot slug={task.slug} />
        <span className="font-semibold text-ink-1">{task.agentLabel}</span>
        <StatusBadge status={task.status} />
        <div className="flex-1" />
        <span className="text-[12px] text-ink-3">
          {formatRelative(task.startedAt, now)}
        </span>
        {/* No translate on hover; the slide animation near the hero
            screenshot was reading as motion of the image itself. */}
        <ChevronRight className="size-4 text-ink-3" />
      </header>

      <div className="mt-3 flex items-center gap-4">
        {task.lastScreenshotDispatchId !== null ? (
          <div className="w-32 shrink-0 transform-gpu overflow-hidden rounded-md border border-border-2 bg-bg-sunken">
            <AspectRatio ratio={16 / 10}>
              {screenshotBaseUrl !== null ? (
                <img
                  src={taskScreenshotUrl(
                    task.lastScreenshotDispatchId,
                    screenshotBaseUrl,
                  )}
                  alt={`Hero from ${task.agentLabel}`}
                  className="h-full w-full object-cover"
                  loading="lazy"
                  decoding="async"
                />
              ) : (
                <div className="h-full w-full animate-pulse bg-card-tint" />
              )}
            </AspectRatio>
          </div>
        ) : (
          <div className="flex w-32 shrink-0 items-center justify-center rounded-md border border-border-2 border-dashed bg-bg-sunken text-[10.5px] text-ink-3 uppercase">
            <span className="px-2 text-center">no screenshot</span>
          </div>
        )}
        <div className="min-w-0 flex-1 space-y-1.5">
          <h3 className="truncate font-semibold text-ink-1">{task.title}</h3>
          <div className="flex items-center gap-3 text-[12.5px] text-ink-3">
            <span className="font-mono">{formatDuration(task.durationMs)}</span>
            <span className="text-ink-3">•</span>
            <span>
              {task.dispatchCount} tool{task.dispatchCount === 1 ? '' : 's'}
            </span>
            {task.errorCount > 0 && (
              <>
                <span className="text-ink-3">•</span>
                <span className="text-red-600 dark:text-red-400">
                  {task.errorCount} error{task.errorCount === 1 ? '' : 's'}
                </span>
              </>
            )}
          </div>
          <p className="truncate font-mono text-[11.5px] text-ink-3">
            {abbreviateSequence(task.toolSequence)}
          </p>
        </div>
      </div>
    </NavLink>
  )
}
