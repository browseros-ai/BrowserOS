/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { SessionSummary } from '@browseros/claw-api'
import { ChevronsDownUp, ChevronsUpDown, Globe, List } from 'lucide-react'
import { useState } from 'react'
import { CodeBlock } from '@/components/agents/code-block'
import { CopyButton } from '@/components/copy/CopyButton'
import { Button } from '@/components/motion/button/base'
import {
  CenterMorphModal,
  CenterMorphModalContent,
  CenterMorphModalTrigger,
} from '@/components/motion/center-morph-modal'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleIndicator,
  CollapsibleTrigger,
} from '@/components/motion/collapsible'
import { Tabs, TabsList, TabsTrigger } from '@/components/motion/tabs'
import { describeStep, isBehindTheScenes } from '@/lib/step-sentences'
import { cn } from '@/lib/utils'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import {
  formatStepDuration,
  logEntryText,
  preciseOffset,
} from './step-log.helpers'

interface StepLogDialogProps {
  session: SessionSummary
  dispatches: readonly ToolDispatchRow[]
}

type Filter = 'all' | 'main'

const END_LABEL: Record<SessionSummary['status'], string> = {
  live: 'Still running',
  done: 'Task finished',
  cancelled: 'You stopped the task',
  failed: 'Task didn’t finish',
}

/** Every recorded call with its exact details, opened on purpose from Technical details. */
export function StepLogDialog({ session, dispatches }: StepLogDialogProps) {
  const [filter, setFilter] = useState<Filter>('all')
  const [openIds, setOpenIds] = useState<ReadonlySet<number>>(new Set())
  const main = dispatches.filter((row) => !isBehindTheScenes(row))
  const rows = filter === 'main' ? main : dispatches
  const allOpen =
    rows.length > 0 && rows.every((row) => openIds.has(row.dispatchId))

  const setRowOpen = (dispatchId: number, open: boolean) =>
    setOpenIds((previous) => {
      const next = new Set(previous)
      if (open) next.add(dispatchId)
      else next.delete(dispatchId)
      return next
    })

  return (
    <CenterMorphModal>
      <CenterMorphModalTrigger>
        <Button
          variant="secondary"
          size="sm"
          className="h-[34px] font-semibold text-[13.5px]"
        >
          <List aria-hidden className="size-[15px]" />
          Show the full step log
        </Button>
      </CenterMorphModalTrigger>
      <CenterMorphModalContent
        ariaLabel="Full step log"
        showCloseButton
        className="max-w-[920px] rounded-[22px] border-border-2 bg-card"
      >
        <div className="flex max-h-[min(820px,calc(100dvh-64px))] flex-col">
          <header className="flex flex-col gap-1 px-6 pt-6 pr-16 pb-4">
            <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
              Full step log
            </h2>
            <p className="text-[14px] text-ink-2">
              Everything neo recorded for this task, with the exact details your
              AI agent sent.
            </p>
          </header>
          <div className="flex flex-wrap items-center gap-2 border-border-2 border-y bg-bg-canvas px-6 py-2.5">
            <Tabs
              variant="segment"
              value={filter}
              onValueChange={(value) => setFilter(value as Filter)}
            >
              <TabsList aria-label="Which steps">
                <TabsTrigger value="all">
                  All steps · {dispatches.length}
                </TabsTrigger>
                <TabsTrigger value="main">
                  Main steps · {main.length}
                </TabsTrigger>
              </TabsList>
            </Tabs>
            <span className="flex-1" />
            <Button
              variant="ghost"
              size="sm"
              onClick={() =>
                setOpenIds(
                  allOpen
                    ? new Set()
                    : new Set(rows.map((row) => row.dispatchId)),
                )
              }
              className="h-8 font-semibold text-[13px]"
            >
              {allOpen ? (
                <ChevronsDownUp aria-hidden className="size-[15px]" />
              ) : (
                <ChevronsUpDown aria-hidden className="size-[15px]" />
              )}
              {allOpen ? 'Collapse all' : 'Expand all'}
            </Button>
            <CopyButton
              text={rows
                .map((row) => logEntryText(row, session.startedAt))
                .join('\n\n')}
              label="Copy all"
              size="sm"
              className="h-8 font-semibold text-[13px]"
            />
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto px-3 py-2">
            {rows.map((row) => (
              <LogRow
                key={row.dispatchId}
                row={row}
                startedAt={session.startedAt}
                open={openIds.has(row.dispatchId)}
                onOpenChange={(open) => setRowOpen(row.dispatchId, open)}
              />
            ))}
            <p className="flex items-center gap-3 px-3 py-3 text-[14px] text-ink-2">
              <span aria-hidden className="size-2 rounded-full bg-ink-3" />
              <span className="font-mono text-[12.5px] text-ink-3 tabular-nums">
                {preciseOffset(
                  session.endedAt ?? Date.now(),
                  session.startedAt,
                )}
              </span>
              {END_LABEL[session.status]}
            </p>
          </div>
        </div>
      </CenterMorphModalContent>
    </CenterMorphModal>
  )
}

// ---------------------------------------------------------------------------

function LogRow({
  row,
  startedAt,
  open,
  onOpenChange,
}: {
  row: ToolDispatchRow
  startedAt: number
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const hidden = isBehindTheScenes(row)
  const details = [row.argsJson, row.resultMeta]
    .filter((json): json is string => Boolean(json))
    .map(prettyJson)
    .join('\n\n')

  return (
    <Collapsible
      open={open}
      onOpenChange={onOpenChange}
      className={cn('rounded-xl', open && 'bg-bg-canvas')}
    >
      <CollapsibleTrigger className="grid w-full grid-cols-[64px_minmax(0,auto)_minmax(0,1fr)_auto_16px] items-center gap-3 rounded-xl px-3 py-2.5 text-left outline-none hover:bg-bg-canvas focus-visible:ring-2 focus-visible:ring-ring">
        <span className="font-mono text-[12.5px] text-ink-3 tabular-nums">
          {preciseOffset(row.createdAt, startedAt)}
        </span>
        <span className="truncate rounded-md bg-bg-sunken px-1.5 py-0.5 font-mono text-[12px] text-ink-2">
          {row.toolName}
        </span>
        <span
          className={cn(
            'truncate text-[14px]',
            hidden ? 'text-ink-3' : 'text-cyanotype-ink',
          )}
        >
          {describeStep(row)}
          {hidden ? (
            <span className="ml-2 text-[12px] text-ink-3">
              behind the scenes
            </span>
          ) : null}
        </span>
        <span className="font-mono text-[12.5px] text-ink-3 tabular-nums">
          {formatStepDuration(row.durationMs)}
        </span>
        <CollapsibleIndicator />
      </CollapsibleTrigger>
      <CollapsibleContent contentClassName="flex flex-col gap-2.5 px-3 pb-3">
        {row.url ? (
          <p className="flex items-center gap-2 text-[13.5px] text-ink-2">
            <Globe aria-hidden className="size-[14px] shrink-0" />
            <span className="truncate">{row.title || row.url}</span>
          </p>
        ) : null}
        {details ? (
          <CodeBlock code={details} language="json" maxHeight={280} copyable />
        ) : (
          <p className="text-[13.5px] text-ink-3">No details were recorded.</p>
        )}
      </CollapsibleContent>
    </Collapsible>
  )
}

function prettyJson(json: string): string {
  try {
    return JSON.stringify(JSON.parse(json), null, 2)
  } catch {
    return json
  }
}
