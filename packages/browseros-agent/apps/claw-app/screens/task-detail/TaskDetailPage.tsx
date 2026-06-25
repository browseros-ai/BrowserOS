import { useState } from 'react'
import { useParams } from 'react-router'
import { ScreenshotLightbox } from '@/components/audit/ScreenshotLightbox'
import { ScreenshotStrip } from '@/components/audit/ScreenshotStrip'
import { TaskHeader } from '@/components/audit/TaskHeader'
import { Timeline } from '@/components/audit/Timeline'
import { EmptyState } from '@/components/cockpit/EmptyState'
import { Skeleton } from '@/components/ui/skeleton'
import { useTaskDetailScreenData } from './task-detail.data'

/**
 * Full-page view of one MCP task. Reached from the homepage card
 * click or the audit row click at `/audit/:sessionId`. Layout:
 *
 *   - TaskHeader     header card with agent, status, timestamps,
 *                    primary actions
 *   - ScreenshotStrip horizontal gallery of every screenshot the
 *                    task captured (clicks open the lightbox)
 *   - Timeline       vertical rail of every dispatch (HIGH RISK
 *                    rows auto-expand)
 *   - Lightbox       shadcn Dialog for the full-size view
 */
export function TaskDetailPage() {
  const { sessionId = '' } = useParams()
  const { task, isPending, isError, error } = useTaskDetailScreenData(sessionId)
  const [lightboxId, setLightboxId] = useState<number | null>(null)

  if (isPending) {
    return (
      <div className="mx-auto w-full max-w-5xl space-y-6 px-8 pt-10 pb-20">
        <Skeleton className="h-6 w-32" />
        <Skeleton className="h-40 w-full rounded-2xl" />
        <Skeleton className="h-44 w-full rounded-2xl" />
        <Skeleton className="h-96 w-full rounded-2xl" />
      </div>
    )
  }
  if (isError || !task) {
    return (
      <div className="mx-auto w-full max-w-5xl px-8 pt-10 pb-20">
        <EmptyState
          title="Task not found"
          hint={
            error?.message ??
            'No dispatches for this session id. It may have been pruned or never existed.'
          }
        />
      </div>
    )
  }

  return (
    <div className="mx-auto w-full max-w-5xl space-y-6 px-8 pt-10 pb-20">
      <TaskHeader task={task} />
      <ScreenshotStrip
        dispatches={task.dispatches}
        screenshotDispatchIds={task.screenshotDispatchIds}
        startedAt={task.startedAt}
        onSelect={setLightboxId}
      />
      <Timeline
        dispatches={task.dispatches}
        startedAt={task.startedAt}
        endEvent={task.endEvent}
        onScreenshotClick={setLightboxId}
      />
      <ScreenshotLightbox
        dispatchId={lightboxId}
        onClose={() => setLightboxId(null)}
      />
    </div>
  )
}
