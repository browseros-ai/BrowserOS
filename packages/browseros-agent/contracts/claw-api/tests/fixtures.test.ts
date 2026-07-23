import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import type { CockpitStats } from '@browseros/claw-api'
import { parse } from 'yaml'

const fixturesDir = join(import.meta.dir, '../fixtures')
const contractDir = join(import.meta.dir, '..')
const retiredLatestScreenshotKey = ['lastScreenshot', 'DispatchId'].join('')
const retiredPreviewTimestampKey = ['preview', 'CapturedAt'].join('')

function fixture(name: string): unknown {
  return JSON.parse(readFileSync(join(fixturesDir, name), 'utf8'))
}

function yaml(name: string): Record<string, unknown> {
  return parse(readFileSync(join(contractDir, name), 'utf8')) as Record<
    string,
    unknown
  >
}

function jsonRoundTrip<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
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

describe('cockpit stats API fixture', () => {
  test('matches the schema and preserves signed values through typed JSON', () => {
    const stats = fixture('cockpit-stats.json') as CockpitStats
    const schemas = yaml('schemas/cockpit.yaml') as {
      CockpitStats: { required: string[] }
      CockpitStatsWindow: {
        required: string[]
        properties: Record<string, { minimum: number; maximum: number }>
      }
    }

    expect(Object.keys(stats).toSorted()).toEqual(
      schemas.CockpitStats.required.toSorted(),
    )
    expect(stats.hasMeasuredStats).toBe(true)

    for (const window of [stats.allTime, stats.last30Days, stats.last7Days]) {
      expect(Object.keys(window).toSorted()).toEqual(
        schemas.CockpitStatsWindow.required.toSorted(),
      )
      for (const field of schemas.CockpitStatsWindow.required) {
        const value = window[field as keyof typeof window]
        const fieldSchema = schemas.CockpitStatsWindow.properties[field]
        expect(Number.isSafeInteger(value)).toBe(true)
        expect(value).toBeGreaterThanOrEqual(fieldSchema?.minimum ?? Number.NaN)
        expect(value).toBeLessThanOrEqual(fieldSchema?.maximum ?? Number.NaN)
      }
    }

    expect(stats.allTime.rawTokenSavingsEstimate).toBeLessThan(0)
    expect(stats.last7Days).toEqual({
      browserClawTokenEstimate: 0,
      screenshotFirstTokenEstimate: 0,
      rawTokenSavingsEstimate: 0,
      humanTimeSavedMs: 0,
      sessionCount: 0,
      toolCallCount: 0,
    })
    expect(jsonRoundTrip<CockpitStats>(stats)).toEqual(stats)
  })
})
