/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * @deprecated Replaced by `FileCardStrip` in
 * `agent-conversation.file-card-strip.tsx`. Kept temporarily so
 * any in-flight callers don't fail to import; remove in a
 * follow-up once nothing external references it.
 *
 * Compact "Files produced" card rendered under an assistant turn.
 */

import { FileText, Image as ImageIcon, Paperclip } from 'lucide-react'
import { type FC, useMemo, useState } from 'react'
import { Button } from '@/components/ui/button'
import { basenameOf, formatFileSize, inferFileKind } from '@/lib/agent-files'
import { cn } from '@/lib/utils'
import { FilePreviewSheet } from './agent-conversation.file-preview-sheet'

export interface ProducedFileLike {
  id: string
  path: string
  size: number
}

interface ArtifactCardProps {
  files: ReadonlyArray<ProducedFileLike>
  className?: string
}

const MAX_INLINE_ROWS = 4

export const ArtifactCard: FC<ArtifactCardProps> = ({ files, className }) => {
  const [openFileId, setOpenFileId] = useState<string | null>(null)
  const [expanded, setExpanded] = useState(false)

  const sortedFiles = useMemo(
    () => [...files].sort((a, b) => a.path.localeCompare(b.path)),
    [files],
  )

  if (sortedFiles.length === 0) return null

  const visible = expanded ? sortedFiles : sortedFiles.slice(0, MAX_INLINE_ROWS)
  const hiddenCount = sortedFiles.length - visible.length
  const openFile = sortedFiles.find((file) => file.id === openFileId) ?? null

  return (
    <div
      className={cn(
        'rounded-xl border border-border/60 bg-card/50 px-3 py-2.5',
        className,
      )}
    >
      <div className="mb-2 flex items-center gap-2 text-muted-foreground text-xs">
        <Paperclip className="size-3.5" />
        <span className="font-medium text-foreground">
          {sortedFiles.length === 1
            ? '1 file produced'
            : `${sortedFiles.length} files produced`}
        </span>
      </div>

      <ul className="flex flex-col gap-1">
        {visible.map((file) => (
          <li key={file.id}>
            <ArtifactRow file={file} onOpen={() => setOpenFileId(file.id)} />
          </li>
        ))}
      </ul>

      {hiddenCount > 0 ? (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="mt-1.5 h-7 px-2 text-xs"
          onClick={() => setExpanded(true)}
        >
          Show {hiddenCount} more
        </Button>
      ) : null}

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

function ArtifactRow({
  file,
  onOpen,
}: {
  file: ProducedFileLike
  onOpen: () => void
}) {
  const name = basenameOf(file.path)
  const kind = inferFileKind(file.path)
  const Icon = kind === 'image' ? ImageIcon : FileText

  return (
    <button
      type="button"
      onClick={onOpen}
      className={cn(
        'flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors',
        'hover:bg-accent/60 focus:bg-accent/60 focus:outline-hidden',
      )}
    >
      <Icon className="size-3.5 shrink-0 text-muted-foreground" />
      <span className="min-w-0 flex-1 truncate font-medium">{name}</span>
      <span className="shrink-0 text-muted-foreground text-xs tabular-nums">
        {formatFileSize(file.size)}
      </span>
    </button>
  )
}
