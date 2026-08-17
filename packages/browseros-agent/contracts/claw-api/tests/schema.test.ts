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
    // Optional token-consumption totals: present on the summary shape, never required
    // (absent for legacy/unmeasured sessions), each a JavaScript-safe unsigned integer.
    const unsignedInteger = {
      type: 'integer',
      format: 'int64',
      minimum: 0,
      maximum: 9_007_199_254_740_991,
    }
    expect(schemas.SessionSummary?.properties).toHaveProperty('tokenUsage')
    expect(schemas.SessionSummary?.required ?? []).not.toContain('tokenUsage')
    expect(schemas.SessionTokenUsage?.required).toEqual([
      'inputTokenEstimate',
      'outputTokenEstimate',
      'totalTokenEstimate',
    ])
    expect(schemas.SessionTokenUsage?.properties).toMatchObject({
      inputTokenEstimate: unsignedInteger,
      outputTokenEstimate: unsignedInteger,
      totalTokenEstimate: unsignedInteger,
    })
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

describe('skills API schema', () => {
  test('registers skill and directory routes and DTOs', () => {
    const openapi = yaml('openapi.yaml') as {
      paths: Record<string, unknown>
      components: {
        parameters: Record<string, unknown>
        schemas: Record<string, unknown>
      }
    }

    expect(openapi.paths).toMatchObject({
      '/api/v1/skills': { $ref: './paths/skills.yaml#/collection' },
      '/api/v1/skills/{name}': { $ref: './paths/skills.yaml#/item' },
      '/api/v1/skills/{name}/runs': { $ref: './paths/skills.yaml#/runs' },
      '/api/v1/directory': { $ref: './paths/skills.yaml#/directory' },
      '/api/v1/directory/{name}': {
        $ref: './paths/skills.yaml#/directoryItem',
      },
      '/api/v1/directory/{name}/add': {
        $ref: './paths/skills.yaml#/directoryAdd',
      },
    })
    expect(openapi.components.parameters).toMatchObject({
      SkillName: { $ref: './parameters.yaml#/SkillName' },
    })
    expect(openapi.components.schemas).toMatchObject({
      SkillOrigin: { $ref: './schemas/skills.yaml#/SkillOrigin' },
      Skill: { $ref: './schemas/skills.yaml#/Skill' },
      SkillList: { $ref: './schemas/skills.yaml#/SkillList' },
      SkillDetail: { $ref: './schemas/skills.yaml#/SkillDetail' },
      SkillRun: { $ref: './schemas/skills.yaml#/SkillRun' },
      SkillRunList: { $ref: './schemas/skills.yaml#/SkillRunList' },
      SkillCreate: { $ref: './schemas/skills.yaml#/SkillCreate' },
      SkillUpdate: { $ref: './schemas/skills.yaml#/SkillUpdate' },
      DirectorySkill: { $ref: './schemas/skills.yaml#/DirectorySkill' },
      DirectorySkillList: {
        $ref: './schemas/skills.yaml#/DirectorySkillList',
      },
      DirectorySkillDetail: {
        $ref: './schemas/skills.yaml#/DirectorySkillDetail',
      },
    })
  })

  test('defines skill operations and DTO shapes', () => {
    const paths = yaml('paths/skills.yaml') as Record<
      string,
      Record<string, { operationId?: string }>
    >
    const schemas = yaml('schemas/skills.yaml') as Record<
      string,
      {
        enum?: string[]
        required?: string[]
        properties?: Record<string, unknown>
      }
    >

    expect(paths.collection?.get?.operationId).toBe('listSkills')
    expect(paths.collection?.post?.operationId).toBe('createSkill')
    expect(paths.item?.get?.operationId).toBe('getSkill')
    expect(paths.item?.put?.operationId).toBe('updateSkill')
    expect(paths.item?.delete?.operationId).toBe('deleteSkill')
    expect(paths.runs?.get?.operationId).toBe('listSkillRuns')
    expect(paths.directory?.get?.operationId).toBe('listDirectory')
    expect(paths.directoryItem?.get?.operationId).toBe('getDirectorySkill')
    expect(paths.directoryAdd?.post?.operationId).toBe('addDirectorySkill')

    expect(schemas.SkillOrigin?.enum).toEqual(['agent', 'manual', 'directory'])
    expect(schemas.Skill?.required).toEqual([
      'name',
      'description',
      'origin',
      'version',
      'linkedAgents',
      'runCount',
      'cleanRunCount',
      'createdAt',
      'updatedAt',
    ])
    expect(schemas.SkillRun?.required).toEqual([
      'id',
      'skillName',
      'sessionId',
      'runNumber',
      'agentId',
      'clean',
      'createdAt',
    ])
    expect(schemas.SkillDetail?.required).toEqual([
      'skill',
      'body',
      'runs',
      'tokenSavings',
    ])
    expect(schemas.SkillTokenSavings?.required).toEqual([
      'saved',
      'otherBrowsers',
      'used',
      'measuredRunCount',
    ])
    expect(schemas.DirectorySkill?.required).toEqual([
      'name',
      'category',
      'description',
      'installs',
    ])
  })
})
