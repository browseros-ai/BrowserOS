import { activityCardCaptionTones } from '@/components/cockpit/activityCardTone'
import { cn } from '@/lib/utils'
import {
  type TaskSummary,
  taskScreenshotUrl,
  useTaskScreenshotBaseUrl,
} from '@/modules/api/audit.hooks'
import { formatDuration, formatRelative } from '@/screens/audit/audit.helpers'

interface AuditHoverPreviewProps {
  task: TaskSummary | null
}

/**
 * Fixed-position screenshot preview pinned to the top-right of the
 * viewport. Fades in whenever the operator hovers a row in the audit
 * list; content swaps as they move between rows without an unmount.
 *
 * When a session has no captured screenshot, the panel switches to a
 * typographic composition of the tool sequence so the panel is never
 * a grey placeholder.
 */
export function AuditHoverPreview({ task }: AuditHoverPreviewProps) {
  const screenshotId = task?.latestScreenshotId ?? null
  const screenshotBaseUrl = useTaskScreenshotBaseUrl()
  return (
    <div
      aria-hidden
      className={cn(
        'pointer-events-none fixed top-24 right-6 z-30 flex w-[360px] flex-col overflow-hidden rounded-2xl border border-border-2 bg-bg-sunken shadow-xl backdrop-blur-md transition-opacity duration-150',
        task ? 'opacity-100' : 'opacity-0',
      )}
    >
      <div className="relative aspect-[16/10] w-full overflow-hidden">
        {task && screenshotId !== null && screenshotBaseUrl !== null ? (
          <img
            src={taskScreenshotUrl(
              task.sessionId,
              screenshotId,
              screenshotBaseUrl,
            )}
            alt=""
            className="absolute inset-0 h-full w-full object-cover object-top"
          />
        ) : task && screenshotId !== null ? (
          <div className="absolute inset-0 animate-pulse bg-card-tint" />
        ) : task ? (
          <NoShotComposition task={task} />
        ) : null}
      </div>
      {task && (
        <div
          className={cn(
            'flex flex-col gap-0.5 px-4 py-3',
            activityCardCaptionTones.blue.surface,
          )}
          data-caption-tone="blue"
        >
          <div className="flex items-center gap-2 font-mono text-[10px] text-white/75 uppercase tracking-[0.08em]">
            <span className="truncate text-white/95">{task.label}</span>
            {task.status === 'live' && <LiveChip />}
          </div>
          <p className="truncate font-semibold text-[13px] text-white leading-tight">
            {task.name}
          </p>
          <p className="font-mono text-[10.5px] text-white/65 tabular-nums">
            {formatDuration(task.durationMs)}{' '}
            <span className="text-white/40">·</span> {task.dispatchCount} tools{' '}
            <span className="text-white/40">·</span>{' '}
            {formatRelative(task.startedAt, Date.now())}
          </p>
        </div>
      )}
    </div>
  )
}

function NoShotComposition({ task }: { task: TaskSummary }) {
  const verbs = task.toolSequence.slice(0, 5)
  return (
    <div className="absolute inset-0 bg-gradient-to-br from-accent-tint via-secondary to-muted">
      <div className="pointer-events-none absolute inset-0 flex flex-col justify-center gap-1 pl-6 font-mono text-[22px] text-ink/15 leading-tight tracking-tight">
        {verbs.map((verb, idx) => (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: tool sequence is stable-ordered per session, not a reorderable list
            key={`${verb}-${idx}`}
            style={{ marginLeft: `${idx * 8}px` }}
            className="truncate"
          >
            {verb}
          </span>
        ))}
      </div>
    </div>
  )
}

/** Matches the audit table's static Live pill so both surfaces read the same. */
function LiveChip() {
  return (
    <span className="inline-flex shrink-0 items-center rounded-full bg-cyanotype-live px-2.5 py-[3px] font-sans font-semibold text-[11px] text-cyanotype-live-ink normal-case leading-[14px] tracking-normal">
      Live
    </span>
  )
}
