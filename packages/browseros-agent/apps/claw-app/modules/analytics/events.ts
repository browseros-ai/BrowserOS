/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The cockpit's event catalog. Events are metadata about which screens
 * and controls get used; they never carry user content (no urls,
 * titles, prompts, tool i/o). `track` only accepts scalar properties.
 */

import { capture } from './posthog'

export const AnalyticsEvent = {
  AppOpened: 'app_opened',
  AuditViewed: 'audit_viewed',
  ReplayViewed: 'replay_viewed',
  TaskDetailViewed: 'task_detail_viewed',
  OptOutToggled: 'analytics_opt_out_toggled',
} as const

export type AnalyticsEventName =
  (typeof AnalyticsEvent)[keyof typeof AnalyticsEvent]

export function track(
  event: AnalyticsEventName,
  properties?: Record<string, boolean | number | string>,
): void {
  capture(event, properties)
}

/**
 * Maps a cockpit route to its view event. The audit table, a task
 * detail, and a replay are the surfaces we care about; the home and
 * mcp routes are covered by `app_opened`.
 */
export function screenEventForPath(path: string): AnalyticsEventName | null {
  if (path === '/audit') return AnalyticsEvent.AuditViewed
  if (/^\/audit\/[^/]+\/replay$/.test(path)) return AnalyticsEvent.ReplayViewed
  if (/^\/audit\/[^/]+$/.test(path)) return AnalyticsEvent.TaskDetailViewed
  return null
}
