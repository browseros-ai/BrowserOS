import { useMemo, useState } from 'react'
import { useParams } from 'react-router'
import { ScreenshotLightbox } from '@/components/audit/ScreenshotLightbox'
import { TaskHeader } from '@/components/audit/TaskHeader'
import { EmptyState } from '@/components/cockpit/EmptyState'
import { Skeleton } from '@/components/ui/skeleton'
import {
  AutoHideTabs,
  type AutoHideTabsItem,
} from '@/components/ui/tabs-auto-hide'
import { TabView } from './TabView'
import { useTaskDetailScreenData } from './task-detail.data'
import { groupDispatchesByTab, pickDefaultTabId } from './task-detail.helpers'

/**
 * Full-page view of one MCP task. Reached from the homepage card
 * click or the audit row click at `/audit/:sessionId`. Layout:
 *
 *   - TaskHeader     header card with agent, status, timestamps,
 *                    primary actions
 *   - AutoHideTabs   one tab per distinct pageId plus a leftmost
 *                    "Session" tab for pageId-less dispatches. When
 *                    the task touched exactly one bucket the tab
 *                    bar hides and the single view renders inline.
 *   - Lightbox       shadcn Dialog for the full-size screenshot
 */
export function TaskDetailPage() {
  const { sessionId = '' } = useParams()
  const { task, isPending, isError, error } = useTaskDetailScreenData(sessionId)
  const [lightboxId, setLightboxId] = useState<number | null>(null)

  const groups = useMemo(
    () =>
      task
        ? groupDispatchesByTab(task.dispatches, task.screenshotDispatchIds)
        : [],
    [task],
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

  const items: AutoHideTabsItem[] = groups.map((g) => ({
    id: g.id,
    label: (
      <span className="inline-flex items-center gap-1.5">
        <span>{g.label}</span>
        <span className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10.5px] text-ink-3">
          {g.dispatchCount}
        </span>
      </span>
    ),
    content: (
      <TabView
        group={g}
        startedAt={task.startedAt}
        endEvent={task.endEvent}
        onScreenshotClick={setLightboxId}
      />
    ),
  }))

  return (
    <div className="mx-auto w-full max-w-5xl space-y-6 px-8 pt-10 pb-20">
      <TaskHeader task={task} />
      <AutoHideTabs
        items={items}
        defaultId={pickDefaultTabId(groups)}
        listVariant="line"
      />
      <ScreenshotLightbox
        dispatchId={lightboxId}
        onClose={() => setLightboxId(null)}
      />
    </div>
  )
}
