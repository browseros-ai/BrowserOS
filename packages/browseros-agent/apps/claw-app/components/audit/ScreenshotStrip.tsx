import type { SessionScreenshot } from '@browseros/claw-api'
import { useMemo } from 'react'
import { AspectRatio } from '@/components/ui/aspect-ratio'
import { ScrollArea, ScrollBar } from '@/components/ui/scroll-area'
import {
  type ToolDispatchRow,
  taskScreenshotUrl,
  useTaskScreenshotBaseUrl,
} from '@/modules/api/audit.hooks'
import { formatOffset, hostOf } from './screenshot.helpers'

interface ScreenshotStripProps {
  sessionId: string
  dispatches: ToolDispatchRow[]
  screenshots: SessionScreenshot[]
  startedAt: number
  onSelect: (screenshotId: number) => void
}

/**
 * Horizontally scrollable strip of every screenshot the task
 * produced, in chronological order. Click any thumb to open the
 * lightbox. Collapses to a one-line placeholder when the task has
 * zero screenshots.
 */
export function ScreenshotStrip({
  sessionId,
  dispatches,
  screenshots,
  startedAt,
  onSelect,
}: ScreenshotStripProps) {
  const screenshotBaseUrl = useTaskScreenshotBaseUrl()
  const meta = useMemo(() => {
    const byScreenshotId = new Map(
      dispatches.flatMap((dispatch) =>
        dispatch.screenshotId === undefined
          ? []
          : [[dispatch.screenshotId, dispatch] as const],
      ),
    )
    return screenshots.map((screenshot) => {
      const dispatch = byScreenshotId.get(screenshot.screenshotId)
      return {
        id: screenshot.screenshotId,
        offset: Math.max(0, screenshot.capturedAt - startedAt),
        url: dispatch?.url ?? null,
      }
    })
  }, [dispatches, screenshots, startedAt])

  if (meta.length === 0) {
    return (
      <div className="rounded-2xl border border-border-2 bg-card px-4 py-3 text-ink-3 text-sm">
        No screenshots captured for this task.
      </div>
    )
  }

  return (
    <section className="rounded-2xl border border-border-2 bg-card p-3">
      <header className="flex items-center justify-between px-1 pb-2 text-[12.5px] text-ink-3">
        <span>
          Screenshots{' '}
          <span className="font-mono text-ink-2">({meta.length})</span>
        </span>
      </header>
      <ScrollArea className="w-full">
        <div className="flex gap-3 pb-2">
          {meta.map((s, idx) => (
            <button
              key={s.id}
              type="button"
              onClick={() => onSelect(s.id)}
              disabled={screenshotBaseUrl === null}
              className="group w-48 shrink-0 text-left"
              data-testid={`screenshot-thumb-${s.id}`}
            >
              <AspectRatio
                ratio={16 / 10}
                className="overflow-hidden rounded-lg border border-border-2 bg-bg-sunken transition group-hover:border-accent"
              >
                {screenshotBaseUrl !== null ? (
                  <img
                    src={taskScreenshotUrl(sessionId, s.id, screenshotBaseUrl)}
                    alt={`Screenshot ${idx + 1}`}
                    className="h-full w-full object-cover"
                    loading="lazy"
                  />
                ) : (
                  <div className="h-full w-full animate-pulse bg-card-tint" />
                )}
              </AspectRatio>
              <div className="mt-1.5 flex items-center justify-between gap-2 text-[11.5px]">
                <span className="font-mono text-ink-3">
                  T+{formatOffset(s.offset)}
                </span>
                <span className="truncate text-ink-3">{hostOf(s.url)}</span>
              </div>
            </button>
          ))}
        </div>
        <ScrollBar orientation="horizontal" />
      </ScrollArea>
    </section>
  )
}
