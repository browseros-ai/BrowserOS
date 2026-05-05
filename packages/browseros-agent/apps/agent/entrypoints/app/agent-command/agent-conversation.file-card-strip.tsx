/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * "Files produced" strip rendered at the bottom of any assistant
 * turn that produced files (openclaw only). Replaces Phase 5.3's
 * row-list ArtifactCard with small horizontal cards for a lighter
 * visual treatment.
 *
 * Click semantics:
 *  - Card  → opens FilePreviewSheet directly (preview + download).
 *  - View  → emits onOpenRail(turnId); the parent opens the rail
 *            and scrolls to the matching turn group.
 *  - +N    → same as View (the user is asking to see what was
 *            overflowed).
 */

import { ChevronRight, FileText, Image as ImageIcon } from 'lucide-react'
import { type FC, useMemo, useState } from 'react'
import { Button } from '@/components/ui/button'
import { basenameOf, formatFileSize, inferFileKind } from '@/lib/agent-files'
import { cn } from '@/lib/utils'
import { FilePreviewSheet } from './agent-conversation.file-preview-sheet'

export interface CardStripFile {
  id: string
  path: string
  size: number
}

interface FileCardStripProps {
  /**
   * The turn id that produced these files. Forwarded to
   * `onOpenRail` so the rail can scroll/expand the matching group.
   * Optional because the live `produced_files` event lands before
   * the harness has stamped a server-issued turn id on the
   * optimistic turn — in that brief window, View falls back to
   * just opening the rail at the top.
   */
  turnId?: string | null
  files: ReadonlyArray<CardStripFile>
  /** Caller wires this to `setOutputsRailOpen(true)` + deep-link. */
  onOpenRail: (turnId?: string | null) => void
  className?: string
}

const MAX_VISIBLE = 4

export const FileCardStrip: FC<FileCardStripProps> = ({
  turnId,
  files,
  onOpenRail,
  className,
}) => {
  const [openFileId, setOpenFileId] = useState<string | null>(null)

  const sortedFiles = useMemo(
    () => [...files].sort((a, b) => a.path.localeCompare(b.path)),
    [files],
  )

  if (sortedFiles.length === 0) return null

  const visible = sortedFiles.slice(0, MAX_VISIBLE)
  const hiddenCount = sortedFiles.length - visible.length
  const openFile = sortedFiles.find((file) => file.id === openFileId) ?? null

  return (
    <div
      className={cn(
        'rounded-xl border border-border/60 bg-card/50 px-3 py-2.5',
        className,
      )}
    >
      <div className="mb-2 flex items-center gap-2">
        <span className="text-muted-foreground text-xs">
          {sortedFiles.length === 1
            ? 'File produced'
            : `Files produced (${sortedFiles.length})`}
        </span>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="ml-auto h-7 gap-1 px-2 text-xs"
          onClick={() => onOpenRail(turnId ?? null)}
        >
          View
          <ChevronRight className="size-3" />
        </Button>
      </div>

      <div className="flex flex-wrap gap-2">
        {visible.map((file) => (
          <FileCard
            key={file.id}
            file={file}
            onOpen={() => setOpenFileId(file.id)}
          />
        ))}
        {hiddenCount > 0 ? (
          <button
            type="button"
            onClick={() => onOpenRail(turnId ?? null)}
            className={cn(
              'flex h-[56px] min-w-[56px] shrink-0 items-center justify-center rounded-lg border border-border/60 px-3 text-muted-foreground text-xs',
              'transition-colors hover:border-border hover:bg-accent/40 hover:text-foreground',
              'focus:outline-hidden focus-visible:ring-2 focus-visible:ring-[var(--accent-orange)]',
            )}
            title={`See ${hiddenCount} more in the Outputs rail`}
          >
            +{hiddenCount}
          </button>
        ) : null}
      </div>

      <FilePreviewSheet
        fileId={openFile?.id ?? null}
        filePath={openFile?.path ?? null}
        open={Boolean(openFileId)}
        onOpenChange={(next) => {
          if (!next) setOpenFileId(null)
        }}
      />
    </div>
  )
}

function FileCard({
  file,
  onOpen,
}: {
  file: CardStripFile
  onOpen: () => void
}) {
  const name = basenameOf(file.path)
  const kind = inferFileKind(file.path)
  const Icon = kind === 'image' ? ImageIcon : FileText

  return (
    <button
      type="button"
      onClick={onOpen}
      title={file.path}
      className={cn(
        'flex h-[56px] w-[140px] shrink-0 flex-col justify-between rounded-lg border border-border/60 bg-background px-2.5 py-1.5 text-left',
        'transition-colors hover:border-border hover:bg-accent/40',
        'focus:outline-hidden focus-visible:ring-2 focus-visible:ring-[var(--accent-orange)]',
      )}
    >
      <div className="flex min-w-0 items-center gap-1.5">
        <Icon className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate font-medium text-xs">
          {name}
        </span>
      </div>
      <span className="text-[11px] text-muted-foreground tabular-nums">
        {formatFileSize(file.size)}
      </span>
    </button>
  )
}
