import { useMemo, useState } from 'react'
import { useParams } from 'react-router'
import {
  ScreenshotLightbox,
  type ScreenshotLightboxItem,
} from '@/components/audit/ScreenshotLightbox'
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
  const { detail, screenshots, isPending, isError, error } =
    useTaskDetailScreenData(sessionId)
  const [openScreenshotId, setOpenScreenshotId] = useState<number | null>(null)

  const groups = useMemo(
    () => (detail ? groupDispatchesByTab(detail.dispatches, screenshots) : []),
    [detail, screenshots],
  )

  const lightboxItems = useMemo<ScreenshotLightboxItem[]>(() => {
    if (!detail) return []
    const urlByScreenshot = new Map<number, string | null>()
    for (const dispatch of detail.dispatches) {
      if (dispatch.screenshotId != null) {
        urlByScreenshot.set(dispatch.screenshotId, dispatch.url ?? null)
      }
    }
    return [...screenshots]
      .sort((a, b) => a.capturedAt - b.capturedAt)
      .map((screenshot) => ({
        screenshotId: screenshot.screenshotId,
        sourceUrl: urlByScreenshot.get(screenshot.screenshotId) ?? null,
        offsetMs: Math.max(0, screenshot.capturedAt - detail.session.startedAt),
      }))
  }, [detail, screenshots])

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
        sessionId={sessionId}
        group={g}
        startedAt={session.startedAt}
        endEvent={endEvent}
        onScreenshotClick={(screenshotId) => setOpenScreenshotId(screenshotId)}
      />
    ),
  }))

  return (
    <div className="mx-auto w-full max-w-5xl space-y-6 px-8 pt-10 pb-20">
      <TaskHeader detail={detail} />
      <AutoHideTabs
        items={items}
        defaultId={pickDefaultTabId(groups)}
        listVariant="line"
        // Many-tab sessions (one tab per browser page) overflow the fixed
        // page width; scroll the strip horizontally instead of spilling
        // off-screen, keep each trigger at its natural width, and hide the
        // scrollbar so the strip scrolls cleanly with no visible track.
        listClassName="w-full max-w-full justify-start overflow-x-auto [&_button]:shrink-0 [scrollbar-width:none] [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden"
      />
      <ScreenshotLightbox
        sessionId={sessionId}
        items={lightboxItems}
        startId={openScreenshotId}
        onClose={() => setOpenScreenshotId(null)}
      />
    </div>
  )
}
