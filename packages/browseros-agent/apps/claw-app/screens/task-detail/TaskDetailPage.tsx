import { useMemo, useState } from 'react'
import { useParams } from 'react-router'
import { ScreenshotLightbox } from '@/components/audit/ScreenshotLightbox'
import { TaskHeader } from '@/components/audit/TaskHeader'
import { EmptyState } from '@/components/cockpit/EmptyState'
import { Skeleton } from '@/components/ui/skeleton'
import { TabView } from './TabView'
import {
  TaskViewNavigation,
  type TaskViewNavigationItem,
} from './TaskViewNavigation'
import { useTaskDetailScreenData } from './task-detail.data'
import { groupDispatchesByTab, pickDefaultTabId } from './task-detail.helpers'

/**
 * Full-page view of one MCP task. Reached from the homepage card
 * click or the audit row click at `/audit/:sessionId`. Layout:
 *
 *   - TaskHeader     header card with agent, status, timestamps,
 *                    primary actions
 *   - TaskViewNavigation  compact selector for the aggregate session
 *                         and each distinct pageId. A sole session
 *                         view renders directly without navigation.
 *   - Lightbox       shadcn Dialog for the full-size screenshot
 */
export function TaskDetailPage() {
  const { sessionId = '' } = useParams()
  const { detail, screenshots, isPending, isError, error } =
    useTaskDetailScreenData(sessionId)
  const [lightboxId, setLightboxId] = useState<number | null>(null)

  const groups = useMemo(
    () => (detail ? groupDispatchesByTab(detail.dispatches, screenshots) : []),
    [detail, screenshots],
  )

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
  if (isError || !detail) {
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

  const selectedDispatch =
    lightboxId !== null
      ? (detail.dispatches.find((d) => d.screenshotId === lightboxId) ?? null)
      : null
  const selectedScreenshot =
    lightboxId !== null
      ? (screenshots.find((s) => s.screenshotId === lightboxId) ?? null)
      : null

  const { session } = detail
  const endEvent = session.endedAt
    ? {
        createdAt: session.endedAt,
        kind:
          session.status === 'failed'
            ? ('errored' as const)
            : session.status === 'cancelled'
              ? ('cancelled' as const)
              : ('closed' as const),
        reason: null,
      }
    : null

  const items: TaskViewNavigationItem[] = groups.map((g) => ({
    id: g.id,
    label: g.label,
    count: g.dispatchCount,
    content: (
      <TabView
        sessionId={sessionId}
        group={g}
        startedAt={session.startedAt}
        endEvent={endEvent}
        onScreenshotClick={setLightboxId}
      />
    ),
  }))

  return (
    <div className="mx-auto w-full max-w-5xl space-y-6 px-8 pt-10 pb-20">
      <TaskHeader detail={detail} />
      <TaskViewNavigation
        key={sessionId}
        items={items}
        defaultId={pickDefaultTabId(groups)}
      />
      <ScreenshotLightbox
        sessionId={sessionId}
        screenshotId={lightboxId}
        sourceUrl={selectedDispatch?.url ?? null}
        offsetMs={
          selectedScreenshot
            ? Math.max(0, selectedScreenshot.capturedAt - session.startedAt)
            : null
        }
        onClose={() => setLightboxId(null)}
      />
    </div>
  )
}
