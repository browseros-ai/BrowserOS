import { AgentDot } from '@/components/audit/AgentDot'
import { cn } from '@/lib/utils'
import { type TaskSummary, taskScreenshotUrl } from '@/modules/api/audit.hooks'
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
 * typographic composition of the tool sequence (same treatment as
 * the cockpit's SupportingTile no-shot variant) so the panel is
 * never a grey placeholder.
 */
export function AuditHoverPreview({ task }: AuditHoverPreviewProps) {
  const screenshotId = task?.lastScreenshotDispatchId ?? null
  return (
    <div
      aria-hidden
      className={cn(
        'pointer-events-none fixed top-24 right-6 z-30 flex w-[360px] flex-col overflow-hidden rounded-2xl border border-border-2 bg-bg-sunken shadow-xl backdrop-blur-md transition-opacity duration-150',
        task ? 'opacity-100' : 'opacity-0',
      )}
    >
      <div className="relative aspect-[16/10] w-full overflow-hidden">
        {task && screenshotId !== null ? (
          <img
            src={taskScreenshotUrl(screenshotId)}
            alt=""
            className="absolute inset-0 h-full w-full object-cover object-top"
          />
        ) : task ? (
          <NoShotComposition task={task} />
        ) : null}
      </div>
      {task && (
        <div className="flex flex-col gap-0.5 bg-ink px-4 py-3 text-white">
          <div className="flex items-center gap-2 font-mono text-[10px] text-white/75 uppercase tracking-[0.08em]">
            <AgentDot slug={task.slug} />
            <span className="truncate text-white/95">{task.agentLabel}</span>
            {task.status === 'live' && <LiveDot />}
          </div>
          <p className="truncate font-semibold text-[13px] text-white leading-tight">
            {task.title}
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
    <div className="absolute inset-0 bg-gradient-to-br from-ink via-ink-2 to-ink">
      <div className="pointer-events-none absolute inset-0 flex flex-col justify-center gap-1 pl-6 font-mono text-[22px] text-white/15 leading-tight tracking-tight">
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

function LiveDot() {
  return (
    <span className="inline-flex items-center gap-1 text-accent">
      <span
        aria-hidden
        className="inline-block size-1.5 animate-[pulse-dot_1.4s_ease-in-out_infinite] rounded-full bg-accent"
      />
      LIVE
    </span>
  )
}
