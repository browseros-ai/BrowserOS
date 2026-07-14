/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { AnalyticsEvent, screenEventForPath } from './events'

describe('screenEventForPath', () => {
  it('maps the audit table, task detail, and replay routes', () => {
    expect(screenEventForPath('/audit')).toBe(AnalyticsEvent.AuditViewed)
    expect(screenEventForPath('/audit/abc123')).toBe(
      AnalyticsEvent.TaskDetailViewed,
    )
    expect(screenEventForPath('/audit/abc123/replay')).toBe(
      AnalyticsEvent.ReplayViewed,
    )
  })

  it('ignores routes without a view event', () => {
    expect(screenEventForPath('/')).toBeNull()
    expect(screenEventForPath('/mcp')).toBeNull()
  })
})
