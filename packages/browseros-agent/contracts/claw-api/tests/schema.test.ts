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
      { get?: { operationId?: string } }
    >
    const schemas = yaml('schemas/sessions.yaml') as Record<
      string,
      { properties?: Record<string, unknown>; required?: string[] }
    >
    const dispatches = yaml('schemas/dispatches.yaml') as Record<
      string,
      { properties?: Record<string, unknown>; required?: string[] }
    >

    expect(paths.preview?.get?.operationId).toBe('getSessionPreview')
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
  })
})
