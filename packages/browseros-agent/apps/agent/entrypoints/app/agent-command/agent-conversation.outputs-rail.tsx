/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Per-agent right-side "Outputs" panel. Lists every file the harness
 * has attributed to this agent, grouped by the turn that produced
 * them. Click a row to open the shared preview Sheet.
 *
 * Lifecycle:
 *  - Open/closed state is controlled by the parent and persisted via
 *    `useOutputsRailOpen(agentId)` so each agent remembers its
 *    preference independently.
 *  - Data refreshes whenever a turn finishes (the conversation hook
 *    fires `useInvalidateAgentOutputs` from its finally block).
 *  - Manual "Refresh" button is wired to `useRefreshAgentOutputs`
 *    for users who navigate in mid-turn.
 */

import {
  ChevronDown,
  ChevronRight,
  FileText,
  Image as ImageIcon,
  Inbox,
  Loader2,
  PanelRightClose,
  RefreshCw,
} from 'lucide-react'
import { type FC, useEffect, useMemo, useRef, useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import {
  basenameOf,
  formatFileSize,
  inferFileKind,
  type ProducedFilesRailGroup,
  useAgentOutputs,
  useRefreshAgentOutputs,
} from '@/lib/agent-files'
import { cn } from '@/lib/utils'
import { FilePreviewSheet } from './agent-conversation.file-preview-sheet'

interface OutputsRailProps {
  agentId: string
  onClose: () => void
  /**
   * When set, the rail scrolls the matching `RailTurnGroup` into
   * view and force-opens its `Collapsible`. Used by the inline
   * file-card strip's "View" / "+N" deep-link path. Cleared by
   * the parent (via `onFocusTurnConsumed`) once the rail has
   * acknowledged the deep-link so subsequent renders don't keep
   * re-scrolling the same group.
   */
  focusTurnId?: string | null
  onFocusTurnConsumed?: () => void
}

const RAIL_LOCAL_STORAGE_PREFIX = 'browseros:outputs-rail:'

/**
 * Controlled open/close state with per-agent localStorage memory.
 * Returns a tuple compatible with React's useState shape so the
 * parent can pass it straight into the rail without an extra effect.
 */
export function useOutputsRailOpen(
  agentId: string,
): [boolean, (next: boolean) => void] {
  const [open, setOpen] = useState(false)

  useEffect(() => {
    if (typeof window === 'undefined' || !agentId) return
    try {
      const stored = window.localStorage.getItem(
        `${RAIL_LOCAL_STORAGE_PREFIX}${agentId}`,
      )
      setOpen(stored === '1')
    } catch {
      // localStorage may be unavailable (private mode, locked-down
      // contexts) — fall back to closed.
    }
  }, [agentId])

  const update = (next: boolean) => {
    setOpen(next)
    if (typeof window === 'undefined' || !agentId) return
    try {
      window.localStorage.setItem(
        `${RAIL_LOCAL_STORAGE_PREFIX}${agentId}`,
        next ? '1' : '0',
      )
    } catch {
      // Best-effort persistence.
    }
  }

  return [open, update]
}

export const OutputsRail: FC<OutputsRailProps> = ({
  agentId,
  onClose,
  focusTurnId,
  onFocusTurnConsumed,
}) => {
  const { groups, loading, error } = useAgentOutputs(agentId)
  const refresh = useRefreshAgentOutputs(agentId)

  const [openFile, setOpenFile] = useState<{
    id: string
    path: string
  } | null>(null)

  const totalFiles = useMemo(
    () => groups.reduce((sum, group) => sum + group.files.length, 0),
    [groups],
  )

  return (
    <aside className="flex h-full min-h-0 w-full flex-col border-border/50 border-l bg-background">
      <header className="flex shrink-0 items-center gap-2 border-border/50 border-b px-3 py-3">
        <span className="font-semibold text-[13px] uppercase tracking-wide">
          Outputs
        </span>
        {totalFiles > 0 ? (
          <span className="text-muted-foreground text-xs tabular-nums">
            {totalFiles}
          </span>
        ) : null}
        <div className="ml-auto flex items-center gap-1">
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="size-7"
            onClick={() =>
              refresh.mutate(undefined, {
                onError: (err) =>
                  toast.error('Refresh failed', {
                    description:
                      err instanceof Error ? err.message : String(err),
                  }),
              })
            }
            disabled={refresh.isPending}
            title="Refresh"
          >
            {refresh.isPending ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <RefreshCw className="size-3.5" />
            )}
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="size-7"
            onClick={onClose}
            title="Hide outputs"
          >
            <PanelRightClose className="size-3.5" />
          </Button>
        </div>
      </header>

      <ScrollArea className="min-h-0 flex-1">
        <div className="px-2 py-2">
          {loading && groups.length === 0 ? (
            <RailSkeleton />
          ) : error ? (
            <RailError message={error.message} />
          ) : groups.length === 0 ? (
            <RailEmpty />
          ) : (
            <ul className="flex flex-col gap-2">
              {groups.map((group) => (
                <li key={group.turnId}>
                  <RailTurnGroup
                    group={group}
                    focused={
                      Boolean(focusTurnId) && focusTurnId === group.turnId
                    }
                    onFocusConsumed={onFocusTurnConsumed}
                    onOpenFile={(file) =>
                      setOpenFile({ id: file.id, path: file.path })
                    }
                  />
                </li>
              ))}
            </ul>
          )}
        </div>
      </ScrollArea>

      <FilePreviewSheet
        fileId={openFile?.id ?? null}
        filePath={openFile?.path ?? null}
        open={Boolean(openFile)}
        onOpenChange={(next) => {
          if (!next) setOpenFile(null)
        }}
      />
    </aside>
  )
}

function RailTurnGroup({
  group,
  focused,
  onFocusConsumed,
  onOpenFile,
}: {
  group: ProducedFilesRailGroup
  focused: boolean
  onFocusConsumed?: () => void
  onOpenFile: (file: { id: string; path: string }) => void
}) {
  const [open, setOpen] = useState(true)
  const headerLabel = group.turnPrompt.trim() || 'Turn'
  const containerRef = useRef<HTMLDivElement>(null)

  // Deep-link consumption: when the parent passes `focused=true`,
  // expand the collapsible (in case the user had collapsed it
  // earlier) and scroll into view. Fire `onFocusConsumed` so the
  // parent can drop the URL param and we don't re-scroll on every
  // render after that.
  useEffect(() => {
    if (!focused) return
    setOpen(true)
    containerRef.current?.scrollIntoView({
      behavior: 'smooth',
      block: 'nearest',
    })
    onFocusConsumed?.()
  }, [focused, onFocusConsumed])

  return (
    <div ref={containerRef}>
      <Collapsible open={open} onOpenChange={setOpen}>
        <CollapsibleTrigger
          className={cn(
            'flex w-full items-center gap-1.5 rounded-md px-1.5 py-1 text-left text-muted-foreground text-xs',
            'transition-colors hover:bg-accent/40 hover:text-foreground',
          )}
        >
          {open ? (
            <ChevronDown className="size-3 shrink-0" />
          ) : (
            <ChevronRight className="size-3 shrink-0" />
          )}
          <span className="min-w-0 flex-1 truncate font-medium">
            {headerLabel}
          </span>
          <span className="shrink-0 tabular-nums">{group.files.length}</span>
        </CollapsibleTrigger>
        <CollapsibleContent>
          <ul className="mt-1 ml-1 flex flex-col gap-0.5 border-border/40 border-l pl-2">
            {group.files.map((file) => (
              <li key={file.id}>
                <RailFileRow file={file} onOpen={() => onOpenFile(file)} />
              </li>
            ))}
          </ul>
        </CollapsibleContent>
      </Collapsible>
    </div>
  )
}

function RailFileRow({
  file,
  onOpen,
}: {
  file: ProducedFilesRailGroup['files'][number]
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
        'flex w-full items-center gap-2 rounded-md px-1.5 py-1 text-left text-xs transition-colors',
        'hover:bg-accent/60 focus:bg-accent/60 focus:outline-hidden',
      )}
      title={file.path}
    >
      <Icon className="size-3 shrink-0 text-muted-foreground" />
      <span className="min-w-0 flex-1 truncate">{name}</span>
      <span className="shrink-0 text-muted-foreground tabular-nums">
        {formatFileSize(file.size)}
      </span>
    </button>
  )
}

function RailSkeleton() {
  return (
    <div className="flex flex-col gap-2 px-1.5 py-1">
      <Skeleton className="h-4 w-1/2" />
      <Skeleton className="h-4 w-3/4" />
      <Skeleton className="h-4 w-2/3" />
      <Skeleton className="h-4 w-5/6" />
    </div>
  )
}

function RailEmpty() {
  return (
    <div className="mx-2 my-3 flex flex-col items-center gap-1.5 rounded-lg border border-border/60 border-dashed bg-muted/20 px-3 py-6 text-center text-muted-foreground text-xs">
      <Inbox className="size-4" />
      <p className="font-medium">No outputs yet</p>
      <p className="text-[11px] text-muted-foreground/70 leading-snug">
        Files this agent creates will appear here, grouped by the turn that made
        them.
      </p>
    </div>
  )
}

function RailError({ message }: { message: string }) {
  return (
    <div className="mx-2 my-3 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-destructive text-xs">
      {message}
    </div>
  )
}
