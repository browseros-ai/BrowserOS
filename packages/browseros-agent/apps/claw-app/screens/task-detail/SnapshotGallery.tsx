import type { SessionScreenshot } from '@browseros/claw-api'
import {
  ChevronLeft,
  ChevronRight,
  ExternalLink,
  ImageOff,
  Play,
} from 'lucide-react'
import { useRef, useState } from 'react'
import { Button, ButtonLink } from '@/components/motion/button/base'
import { Card } from '@/components/motion/card'
import {
  ImageViewer,
  ImageViewerClose,
  ImageViewerContent,
  ImageViewerCounter,
  ImageViewerGallery,
  ImageViewerNext,
  ImageViewerPrevious,
  ImageViewerThumbnail,
  type LightboxImage,
  useImageViewer,
} from '@/components/motion/morphing-lightbox'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/motion/tooltip'
import { describeStep } from '@/lib/step-sentences'
import {
  type ToolDispatchRow,
  taskScreenshotUrl,
  useTaskScreenshotBaseUrl,
} from '@/modules/api/audit.hooks'
import { offsetLabel } from './task-detail.helpers'

// Screenshots are viewport captures; this keeps the thumbnail morph in shape.
const SHOT_WIDTH = 1280
const SHOT_HEIGHT = 800

interface Snapshot {
  image: LightboxImage
  time: string
  sentence: string
  url?: string
  dispatchId?: number
}

interface SnapshotGalleryProps {
  sessionId: string
  startedAt: number
  screenshots: readonly SessionScreenshot[]
  dispatches: readonly ToolDispatchRow[]
  /** Moves the player to the step that took this picture. */
  onPlayFrom: (dispatchId: number) => void
}

/** Every picture neo took, in one sideways row that opens into a viewer. */
export function SnapshotGallery({
  sessionId,
  startedAt,
  screenshots,
  dispatches,
  onPlayFrom,
}: SnapshotGalleryProps) {
  const baseUrl = useTaskScreenshotBaseUrl()
  const rowRef = useRef<HTMLElement>(null)
  const [openId, setOpenId] = useState<string | null>(null)
  const owners = new Map(
    dispatches.flatMap((row) =>
      row.screenshotId != null ? [[row.screenshotId, row] as const] : [],
    ),
  )
  const snapshots: Snapshot[] =
    baseUrl === null
      ? []
      : [...screenshots]
          .sort((left, right) => left.capturedAt - right.capturedAt)
          .map((shot) => {
            const owner = owners.get(shot.screenshotId)
            const sentence = owner
              ? describeStep(owner)
              : 'A picture of the page'
            return {
              image: {
                id: String(shot.screenshotId),
                src: taskScreenshotUrl(sessionId, shot.screenshotId, baseUrl),
                alt: sentence,
                width: SHOT_WIDTH,
                height: SHOT_HEIGHT,
              },
              time: offsetLabel(shot.capturedAt, startedAt),
              sentence,
              url: owner?.url ?? undefined,
              dispatchId: owner?.dispatchId,
            }
          })
  const byId = new Map(
    snapshots.map((snapshot) => [snapshot.image.id, snapshot]),
  )

  const scrollRow = (direction: -1 | 1) => {
    const row = rowRef.current
    row?.scrollBy({
      left: direction * row.clientWidth * 0.8,
      behavior: 'smooth',
    })
  }

  return (
    <section aria-labelledby="shots-title" className="flex flex-col gap-3.5">
      <div className="flex items-center justify-between gap-4">
        <h2
          id="shots-title"
          className="font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]"
        >
          Snapshots
        </h2>
        {snapshots.length > 0 ? (
          <div className="flex items-center gap-2">
            <span className="text-[13.5px] text-ink-3 tabular-nums">
              {snapshots.length}{' '}
              {snapshots.length === 1 ? 'picture' : 'pictures'}
            </span>
            {snapshots.length > 1 ? (
              <>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="secondary"
                      size="icon"
                      aria-label="Scroll snapshots left"
                      onClick={() => scrollRow(-1)}
                      className="size-9 rounded-full"
                    >
                      <ChevronLeft aria-hidden className="size-4" />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Scroll left</TooltipContent>
                </Tooltip>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="secondary"
                      size="icon"
                      aria-label="Scroll snapshots right"
                      onClick={() => scrollRow(1)}
                      className="size-9 rounded-full"
                    >
                      <ChevronRight aria-hidden className="size-4" />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Scroll right</TooltipContent>
                </Tooltip>
              </>
            ) : null}
          </div>
        ) : null}
      </div>
      {snapshots.length === 0 ? (
        <Card
          variant="muted"
          size="compact"
          className="flex-row items-center gap-4 px-5 py-5"
        >
          <span
            aria-hidden
            className="grid size-11 shrink-0 place-items-center rounded-full bg-accent-tint text-primary"
          >
            <ImageOff className="size-[18px]" />
          </span>
          <span className="flex flex-col gap-0.5">
            <span className="font-bold text-cyanotype-ink">
              No pictures for this task
            </span>
            <span className="text-[14.5px] text-ink-2">
              neo takes pictures as your AI agents work. This task stopped
              before the first one.
            </span>
          </span>
        </Card>
      ) : (
        <ImageViewer
          images={snapshots.map((snapshot) => snapshot.image)}
          value={openId}
          onValueChange={setOpenId}
          label="Snapshots, scroll sideways"
        >
          <ImageViewerGallery
            ref={rowRef}
            className="flex snap-x snap-mandatory gap-3.5 overflow-x-auto pb-2 [scrollbar-width:thin]"
          >
            {snapshots.map((snapshot) => (
              <ImageViewerThumbnail
                key={snapshot.image.id}
                imageId={snapshot.image.id}
                className="flex shrink-0 basis-[260px] snap-start flex-col overflow-hidden rounded-md border border-border-2 bg-card text-left shadow-card"
                imageClassName="aspect-[16/10] object-cover object-top-left"
              >
                <span className="flex flex-col gap-0.5 px-3 py-2.5">
                  <span className="font-mono text-[12px] text-ink-3 tabular-nums">
                    {snapshot.time}
                  </span>
                  <span className="line-clamp-2 text-[13.5px] text-ink">
                    {snapshot.sentence}
                  </span>
                </span>
              </ImageViewerThumbnail>
            ))}
          </ImageViewerGallery>
          <ImageViewerContent
            // Portaled outside the task page's ph-no-capture wrapper, so it
            // blocks the snapshots from session recordings itself.
            className="ph-no-capture"
            header={
              <div className="flex w-full items-center justify-between gap-3 text-white">
                <ImageViewerCounter className="text-sm" />
                <ImageViewerClose />
              </div>
            }
          >
            <ViewerFooter
              byId={byId}
              onPlayFrom={(dispatchId) => {
                setOpenId(null)
                onPlayFrom(dispatchId)
              }}
            />
          </ImageViewerContent>
        </ImageViewer>
      )}
    </section>
  )
}

// ---------------------------------------------------------------------------

function ViewerFooter({
  byId,
  onPlayFrom,
}: {
  byId: ReadonlyMap<string, Snapshot>
  onPlayFrom: (dispatchId: number) => void
}) {
  const { image } = useImageViewer()
  const snapshot = image ? byId.get(image.id) : undefined
  return (
    <div className="flex w-full items-center gap-3 text-white">
      <ImageViewerPrevious />
      <div className="flex min-w-0 flex-1 flex-col items-center gap-2 text-center">
        <p className="min-w-0 text-sm">
          <span className="mr-2 font-mono text-white/70 tabular-nums">
            {snapshot?.time}
          </span>
          {snapshot?.sentence}
        </p>
        <div className="flex flex-wrap justify-center gap-2">
          {snapshot?.dispatchId !== undefined ? (
            <Button
              size="sm"
              onClick={() => {
                if (snapshot.dispatchId !== undefined) {
                  onPlayFrom(snapshot.dispatchId)
                }
              }}
              className="h-8 font-semibold text-[13px]"
            >
              <Play aria-hidden className="size-3.5" />
              Play from here
            </Button>
          ) : null}
          {snapshot?.url ? (
            <ButtonLink
              href={snapshot.url}
              target="_blank"
              rel="noreferrer"
              size="sm"
              variant="outline"
              className="h-8 border-white/30 font-semibold text-[13px] text-white hover:bg-white/10"
            >
              Open the page
              <ExternalLink aria-hidden className="size-3.5" />
            </ButtonLink>
          ) : null}
        </div>
      </div>
      <ImageViewerNext />
    </div>
  )
}
