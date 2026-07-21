import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const fixturesDir = join(import.meta.dir, '../fixtures')
const retiredLatestScreenshotKey = ['lastScreenshot', 'DispatchId'].join('')
const retiredPreviewTimestampKey = ['preview', 'CapturedAt'].join('')

function fixture(name: string): unknown {
  return JSON.parse(readFileSync(join(fixturesDir, name), 'utf8'))
}

describe('session visual API fixtures', () => {
  test('uses resource IDs in session and dispatch DTOs', () => {
    const detail = fixture('session-detail.json') as {
      session: Record<string, unknown>
      dispatches: Array<Record<string, unknown>>
    }
    const list = fixture('session-list.json') as {
      items: Array<{ live?: { browserTabs: Array<Record<string, unknown>> } }>
    }

    expect(detail.session.latestScreenshotId).toBe(42)
    expect(detail.session).not.toHaveProperty(retiredLatestScreenshotKey)
    expect(detail.dispatches[0]).not.toHaveProperty('screenshotId')
    expect(detail.dispatches[1]?.screenshotId).toBe(42)
    expect(
      list.items.flatMap((session) => session.live?.browserTabs ?? []),
    ).not.toContainEqual(
      expect.objectContaining({
        [retiredPreviewTimestampKey]: expect.anything(),
      }),
    )
  })

  test('lists ordered session screenshot metadata without image bytes', () => {
    expect(fixture('session-screenshot-list.json')).toEqual({
      items: [
        { screenshotId: 7, capturedAt: 1000, toolName: 'navigate' },
        { screenshotId: 42, capturedAt: 2000, toolName: 'snapshot' },
      ],
    })
  })
})
