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

describe('skills API fixtures', () => {
  test('skill list carries origin, version, and run stats', () => {
    const list = fixture('skill-list.json') as {
      items: Array<Record<string, unknown>>
    }
    const skill = list.items[0] as Record<string, unknown>
    expect(skill.name).toBe('masason-email-sweep')
    expect(skill.origin).toBe('agent')
    expect(skill.version).toBe(4)
    expect(skill.cleanRunCount).toBe(17)
    expect(Number.isSafeInteger(skill.latestRunTokens as number)).toBe(true)
    expect(jsonRoundTrip(list)).toEqual(list)
  })

  test('skill detail bundles the SKILL.md body and its runs', () => {
    const detail = fixture('skill-detail.json') as {
      skill: Record<string, unknown>
      body: string
      runs: Array<Record<string, unknown>>
    }
    expect(detail.skill.name).toBe('masason-email-sweep')
    expect(detail.body).toContain('name: masason-email-sweep')
    expect(detail.runs.length).toBeGreaterThan(0)
    expect(detail.runs[0]?.clean).toBe(true)
  })

  test('run list records a not-clean run with the failing tool', () => {
    const runs = fixture('skill-run-list.json') as {
      items: Array<Record<string, unknown>>
    }
    const failed = runs.items.find((run) => run.clean === false)
    expect(failed?.erroredTool).toBe('compose')
  })

  test('directory list reports provenance and installs', () => {
    const list = fixture('directory-list.json') as {
      items: Array<Record<string, unknown>>
      sourceRepo?: string
    }
    expect(list.sourceRepo).toBe('browseros-ai/skills')
    const entry = list.items[0] as Record<string, unknown>
    expect(entry.category).toBe('RESEARCH')
    expect(Number.isSafeInteger(entry.installs as number)).toBe(true)
  })
})
