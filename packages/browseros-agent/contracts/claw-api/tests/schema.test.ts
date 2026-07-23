import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { parse } from 'yaml'

const contractDir = join(import.meta.dir, '..')
const retiredBrowserTabPreview = [
  '/api/v1/sessions/{sessionId}',
  'browser-tabs',
  '{browserTabId}',
  'preview',
].join('/')
const retiredDispatchScreenshot = [
  '/api/v1',
  'dispatches',
  '{dispatchId}',
  'screenshot',
].join('/')
const retiredLatestScreenshotKey = ['lastScreenshot', 'DispatchId'].join('')
const retiredPreviewTimestampKey = ['preview', 'CapturedAt'].join('')
const retiredDispatchScreenshotKey = ['has', 'Screenshot'].join('')

function yaml(name: string): Record<string, unknown> {
  return parse(readFileSync(join(contractDir, name), 'utf8')) as Record<
    string,
    unknown
  >
}

describe('session visual API schema', () => {
  test('owns preview and screenshot routes beneath sessions', () => {
    const openapi = yaml('openapi.yaml') as {
      paths: Record<string, unknown>
    }

    expect(openapi.paths).toMatchObject({
      '/api/v1/sessions/{sessionId}/preview': {
        $ref: './paths/sessions.yaml#/preview',
      },
      '/api/v1/sessions/{sessionId}/screenshots': {
        $ref: './paths/sessions.yaml#/screenshots',
      },
      '/api/v1/sessions/{sessionId}/screenshots/{screenshotId}': {
        $ref: './paths/sessions.yaml#/screenshot',
      },
    })
    expect(openapi.paths).not.toHaveProperty(retiredBrowserTabPreview)
    expect(openapi.paths).not.toHaveProperty(retiredDispatchScreenshot)
  })

  test('defines session-owned operations and screenshot DTOs', () => {
    const paths = yaml('paths/sessions.yaml') as Record<
      string,
      {
        get?: {
          operationId?: string
          parameters?: Array<{
            name?: string
            in?: string
            schema?: Record<string, unknown>
          }>
        }
      }
    >
    const schemas = yaml('schemas/sessions.yaml') as Record<
      string,
      {
        enum?: string[]
        properties?: Record<string, unknown>
        required?: string[]
      }
    >
    const dispatches = yaml('schemas/dispatches.yaml') as Record<
      string,
      { properties?: Record<string, unknown>; required?: string[] }
    >

    expect(paths.preview?.get?.operationId).toBe('getSessionPreview')
    expect(paths.preview?.get?.parameters).toContainEqual({
      name: 'refresh',
      in: 'query',
      description: 'Ignored client cache-busting token for preview URLs.',
      schema: { type: 'integer', format: 'int64', minimum: 0 },
    })
    expect(paths.screenshots?.get?.operationId).toBe('listSessionScreenshots')
    expect(paths.screenshot?.get?.operationId).toBe('getSessionScreenshot')
    expect(schemas.SessionScreenshot?.required).toEqual([
      'screenshotId',
      'capturedAt',
      'toolName',
    ])
    expect(schemas.SessionScreenshotList?.required).toEqual(['items'])
    expect(schemas.SessionSummary?.properties).toHaveProperty(
      'latestScreenshotId',
    )
    expect(schemas.SessionSummary?.properties).not.toHaveProperty(
      retiredLatestScreenshotKey,
    )
    expect(schemas.SessionBrowserTab?.properties).not.toHaveProperty(
      retiredPreviewTimestampKey,
    )
    expect(dispatches.Dispatch?.properties).toHaveProperty('screenshotId')
    expect(dispatches.Dispatch?.properties).not.toHaveProperty(
      retiredDispatchScreenshotKey,
    )
    expect(dispatches.Dispatch?.required).not.toContain('screenshotId')
    expect(schemas.SessionStatus?.enum).toEqual([
      'live',
      'done',
      'failed',
      'cancelled',
    ])
    expect(schemas.CancelSessionResponse?.required).toEqual([
      'status',
      'cancelledDispatches',
    ])
    expect(schemas.CancelSessionResponse?.properties).toEqual({
      status: { $ref: '#/SessionStatus' },
      cancelledDispatches: {
        type: 'integer',
        format: 'int64',
        minimum: 0,
      },
    })
  })
})

describe('cockpit stats API schema', () => {
  test('registers one parameterless GET operation', () => {
    const openapi = yaml('openapi.yaml') as {
      paths: Record<string, unknown>
      components: { schemas: Record<string, unknown> }
    }
    const paths = yaml('paths/cockpit.yaml') as Record<
      string,
      Record<string, unknown>
    >

    expect(openapi.paths['/api/v1/cockpit/stats']).toEqual({
      $ref: './paths/cockpit.yaml#/stats',
    })
    expect(openapi.components.schemas).toMatchObject({
      CockpitStats: { $ref: './schemas/cockpit.yaml#/CockpitStats' },
      CockpitStatsWindow: {
        $ref: './schemas/cockpit.yaml#/CockpitStatsWindow',
      },
    })
    expect(Object.keys(paths.stats ?? {})).toEqual(['get'])
    expect(paths.stats?.get).toMatchObject({
      operationId: 'getCockpitStats',
      responses: {
        '200': {
          content: {
            'application/json': {
              schema: { $ref: '../schemas/cockpit.yaml#/CockpitStats' },
            },
          },
        },
      },
    })
    expect(paths.stats?.get).not.toHaveProperty('parameters')
  })

  test('defines exact response fields with safe signed and unsigned integers', () => {
    const schemas = yaml('schemas/cockpit.yaml') as Record<
      string,
      {
        additionalProperties?: boolean
        properties?: Record<string, unknown>
        required?: string[]
      }
    >
    const unsignedInteger = {
      type: 'integer',
      format: 'int64',
      minimum: 0,
      maximum: 9_007_199_254_740_991,
    }

    expect(schemas.CockpitStats).toEqual({
      type: 'object',
      additionalProperties: false,
      required: ['hasMeasuredStats', 'allTime', 'last30Days', 'last7Days'],
      properties: {
        hasMeasuredStats: { type: 'boolean' },
        allTime: { $ref: '#/CockpitStatsWindow' },
        last30Days: { $ref: '#/CockpitStatsWindow' },
        last7Days: { $ref: '#/CockpitStatsWindow' },
      },
    })
    expect(schemas.CockpitStatsWindow).toEqual({
      type: 'object',
      additionalProperties: false,
      required: [
        'browserClawTokenEstimate',
        'screenshotFirstTokenEstimate',
        'rawTokenSavingsEstimate',
        'humanTimeSavedMs',
        'sessionCount',
        'toolCallCount',
      ],
      properties: {
        browserClawTokenEstimate: unsignedInteger,
        screenshotFirstTokenEstimate: unsignedInteger,
        rawTokenSavingsEstimate: {
          type: 'integer',
          format: 'int64',
          minimum: -9_007_199_254_740_991,
          maximum: 9_007_199_254_740_991,
        },
        humanTimeSavedMs: unsignedInteger,
        sessionCount: unsignedInteger,
        toolCallCount: unsignedInteger,
      },
    })
  })
})
