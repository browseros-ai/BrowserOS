/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Renders a single `TabGroup`'s body: a small context header
 * (URL / title), the filtered ScreenshotStrip, and the filtered
 * Timeline. Session groups additionally show the SessionEndRow
 * inside their Timeline; per-page groups hide it because the
 * session end is not scoped to any one tab.
 */

import { useMemo } from 'react'
import { ScreenshotStrip } from '@/components/audit/ScreenshotStrip'
import { Timeline } from '@/components/audit/Timeline'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import type { TabGroup } from './task-detail.helpers'

export interface TabViewProps {
  sessionId: string
  group: TabGroup
  allDispatches?: ToolDispatchRow[]
  startedAt: number
  endEvent: {
    createdAt: number
    kind: 'closed' | 'errored' | 'cancelled'
    reason: string | null
  } | null
  onScreenshotClick: (screenshotId: number) => void
}

export function TabView({
  sessionId,
  group,
  allDispatches = group.dispatches,
  startedAt,
  endEvent,
  onScreenshotClick,
}: TabViewProps) {
  const isSession = group.id === 'session'
  // Script parents have no page, so filtering to a tab removes them. Retain
  // their names as context without adding other tabs' steps to this timeline.
  const parentToolNames = useMemo(
    () =>
      new Map(
        allDispatches.flatMap((row) =>
          row.dispatchKey ? [[row.dispatchKey, row.toolName] as const] : [],
        ),
      ),
    [allDispatches],
  )
  return (
    <div className="space-y-4">
      {(group.displayUrl || group.displayTitle) && (
        <div className="rounded-2xl border border-border-2 bg-card px-4 py-3 text-[12.5px] text-ink-3">
          <div className="font-semibold text-ink">{group.label}</div>
          {group.displayUrl && (
            <div className="truncate font-mono text-[11.5px]">
              {group.displayUrl}
            </div>
          )}
          {group.displayTitle && (
            <div className="truncate">{group.displayTitle}</div>
          )}
        </div>
      )}
      <ScreenshotStrip
        sessionId={sessionId}
        dispatches={group.dispatches}
        screenshots={group.screenshots}
        startedAt={startedAt}
        onSelect={onScreenshotClick}
      />
      <Timeline
        dispatches={group.dispatches}
        startedAt={startedAt}
        endEvent={isSession ? endEvent : null}
        parentToolNames={parentToolNames}
        showSessionEnd={isSession}
        onScreenshotClick={onScreenshotClick}
      />
    </div>
  )
}
