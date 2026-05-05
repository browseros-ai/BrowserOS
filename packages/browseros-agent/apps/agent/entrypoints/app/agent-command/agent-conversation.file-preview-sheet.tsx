/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Shared preview drawer used by the inline artifact card AND the
 * Outputs rail. Branches on the FilePreview discriminated union and
 * renders the appropriate body. Always opens via a controlled
 * `open`/`onOpenChange` pair so the parent owns the selected file.
 */

import { Download, FileWarning, Loader2 } from 'lucide-react'
import { type FC, useEffect, useMemo, useRef } from 'react'
import { toast } from 'sonner'
import { MessageResponse } from '@/components/ai-elements/message'
import { Button } from '@/components/ui/button'
import { ScrollArea } from '@/components/ui/scroll-area'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { Skeleton } from '@/components/ui/skeleton'
import {
  basenameOf,
  buildFileDownloadUrl,
  extensionOf,
  type FilePreview,
  formatFileSize,
  useFilePreview,
} from '@/lib/agent-files'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'
import { cn } from '@/lib/utils'

interface FilePreviewSheetProps {
  fileId: string | null
  filePath: string | null
  open: boolean
  onOpenChange: (open: boolean) => void
}

const MARKDOWN_EXTENSIONS = new Set(['md', 'markdown', 'mdx'])

export const FilePreviewSheet: FC<FilePreviewSheetProps> = ({
  fileId,
  filePath,
  open,
  onOpenChange,
}) => {
  const { baseUrl } = useAgentServerUrl()
  const { preview, loading, error } = useFilePreview(fileId, open)

  const fileName = filePath ? basenameOf(filePath) : 'File preview'
  const downloadUrl = useMemo(() => {
    if (!baseUrl || !fileId) return null
    return buildFileDownloadUrl(baseUrl, fileId)
  }, [baseUrl, fileId])

  // Surface preview-load failures in a toast in addition to the
  // inline error block — the inline UI lives at the bottom of the
  // sheet and is easy to miss when scrolled into the body.
  const lastToastedFileIdRef = useRef<string | null>(null)
  useEffect(() => {
    if (!open) {
      lastToastedFileIdRef.current = null
      return
    }
    if (!error || !fileId) return
    if (lastToastedFileIdRef.current === fileId) return
    lastToastedFileIdRef.current = fileId
    toast.error('Could not load preview', { description: error.message })
  }, [open, error, fileId])

  const handleDownload = () => {
    if (!downloadUrl) {
      toast.error("Couldn't reach the agent server", {
        description: 'Reconnect to BrowserOS and try again.',
      })
      return
    }
    // Manually trigger the download so any future failure (e.g. the
    // server returns 404 because the file was removed) can be
    // surfaced via toast — the bare <a download> path swallows
    // these errors silently.
    const link = document.createElement('a')
    link.href = downloadUrl
    link.download = fileName
    link.rel = 'noopener'
    document.body.appendChild(link)
    link.click()
    link.remove()
  }

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        className="flex w-full flex-col gap-0 p-0 sm:max-w-xl"
      >
        <SheetHeader className="border-border/60 border-b px-5 py-4">
          <SheetTitle className="truncate pr-8">{fileName}</SheetTitle>
          <SheetDescription className="truncate">
            {filePath ?? ''}
          </SheetDescription>
        </SheetHeader>

        <ScrollArea className="min-h-0 flex-1">
          <div className="px-5 py-4">
            {loading ? (
              <PreviewSkeleton />
            ) : error ? (
              <PreviewError message={error.message} />
            ) : preview ? (
              <PreviewBody
                preview={preview}
                filePath={filePath}
                downloadUrl={downloadUrl}
              />
            ) : null}
          </div>
        </ScrollArea>

        {fileId ? (
          <div className="border-border/60 border-t bg-background/90 px-5 py-3 backdrop-blur">
            <Button
              type="button"
              size="sm"
              className="w-full gap-2"
              onClick={handleDownload}
            >
              <Download className="size-3.5" />
              Download
            </Button>
          </div>
        ) : null}
      </SheetContent>
    </Sheet>
  )
}

function PreviewSkeleton() {
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2 text-muted-foreground text-xs">
        <Loader2 className="size-3.5 animate-spin" />
        Loading preview...
      </div>
      <Skeleton className="h-4 w-3/4" />
      <Skeleton className="h-4 w-full" />
      <Skeleton className="h-4 w-5/6" />
      <Skeleton className="h-4 w-2/3" />
    </div>
  )
}

function PreviewError({ message }: { message: string }) {
  return (
    <div className="flex flex-col items-start gap-2 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-destructive text-sm">
      <div className="flex items-center gap-2 font-medium">
        <FileWarning className="size-4" />
        Could not load preview
      </div>
      <p className="text-destructive/80 text-xs">{message}</p>
    </div>
  )
}

function PreviewBody({
  preview,
  filePath,
  downloadUrl,
}: {
  preview: FilePreview
  filePath: string | null
  downloadUrl: string | null
}) {
  if (preview.kind === 'missing') {
    return (
      <div className="rounded-lg border border-border/60 bg-muted/40 px-4 py-6 text-center text-muted-foreground text-sm">
        This file is no longer in the workspace. The agent may have moved or
        deleted it after the turn finished.
      </div>
    )
  }

  if (preview.kind === 'image') {
    return (
      <div className="flex flex-col gap-3">
        <PreviewMeta preview={preview} />
        <div className="overflow-hidden rounded-lg border border-border/60 bg-muted/30">
          <img
            src={preview.dataUrl}
            alt={filePath ?? 'preview'}
            className="block max-h-[60vh] w-full object-contain"
          />
        </div>
      </div>
    )
  }

  if (preview.kind === 'pdf') {
    return (
      <div className="flex flex-col gap-3">
        <PreviewMeta preview={preview} />
        <div className="rounded-lg border border-border/60 bg-muted/40 px-4 py-6 text-center text-muted-foreground text-sm">
          PDF previews aren't supported inline yet. Use Download to open this
          file in your default PDF viewer.
        </div>
      </div>
    )
  }

  if (preview.kind === 'binary') {
    return (
      <div className="flex flex-col gap-3">
        <PreviewMeta preview={preview} />
        <div className="rounded-lg border border-border/60 bg-muted/40 px-4 py-6 text-center text-muted-foreground text-sm">
          No inline preview for this file type.
          {downloadUrl ? ' Use Download to save it locally.' : null}
        </div>
      </div>
    )
  }

  return <TextPreviewBody preview={preview} filePath={filePath} />
}

function TextPreviewBody({
  preview,
  filePath,
}: {
  preview: Extract<FilePreview, { kind: 'text' }>
  filePath: string | null
}) {
  const ext = filePath ? extensionOf(filePath).toLowerCase() : ''
  const renderAsMarkdown = MARKDOWN_EXTENSIONS.has(ext)

  return (
    <div className="flex flex-col gap-3">
      <PreviewMeta preview={preview} />
      {renderAsMarkdown ? (
        <div
          className={cn(
            'prose prose-sm dark:prose-invert max-w-none break-words rounded-lg border border-border/60 bg-muted/30 px-4 py-3',
            "[&_[data-streamdown='code-block']]:!w-full [&_[data-streamdown='code-block']]:overflow-x-auto",
          )}
        >
          <MessageResponse mode="static" parseIncompleteMarkdown={false}>
            {preview.snippet}
          </MessageResponse>
        </div>
      ) : (
        <pre className="overflow-x-auto rounded-lg border border-border/60 bg-muted/30 px-3 py-2 text-xs leading-relaxed">
          <code className="font-mono text-foreground">{preview.snippet}</code>
        </pre>
      )}
      {preview.truncated ? (
        <div className="text-muted-foreground text-xs">
          Showing the first part of this file. Download to see the full
          contents.
        </div>
      ) : null}
    </div>
  )
}

function PreviewMeta({
  preview,
}: {
  preview: Exclude<FilePreview, { kind: 'missing' }>
}) {
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-muted-foreground text-xs">
      <span className="font-medium text-foreground">
        {formatFileSize(preview.size)}
      </span>
      <span>·</span>
      <span className="font-mono">{preview.mimeType || 'unknown'}</span>
    </div>
  )
}
