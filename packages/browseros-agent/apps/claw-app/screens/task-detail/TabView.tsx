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

import { ScreenshotStrip } from '@/components/audit/ScreenshotStrip'
import { Timeline } from '@/components/audit/Timeline'
import type { TabGroup } from './task-detail.helpers'

export interface TabViewProps {
  group: TabGroup
  startedAt: number
  endEvent: {
    createdAt: number
    kind: 'closed' | 'errored'
    reason: string | null
  } | null
  onScreenshotClick: (dispatchId: number) => void
}

export function TabView({
  group,
  startedAt,
  endEvent,
  onScreenshotClick,
}: TabViewProps) {
  const isSession = group.id === 'session'
  return (
    <div className="space-y-4">
      {(group.displayUrl || group.displayTitle) && (
        <div className="rounded-2xl border border-border-2 bg-card px-4 py-3 text-[12.5px] text-ink-3">
          <div className="font-semibold text-ink-1">{group.label}</div>
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
        dispatches={group.dispatches}
        screenshotDispatchIds={group.screenshotDispatchIds}
        startedAt={startedAt}
        onSelect={onScreenshotClick}
      />
      <Timeline
        dispatches={group.dispatches}
        screenshotDispatchIds={group.screenshotDispatchIds}
        startedAt={startedAt}
        endEvent={isSession ? endEvent : null}
        showSessionEnd={isSession}
        onScreenshotClick={onScreenshotClick}
      />
    </div>
  )
}
