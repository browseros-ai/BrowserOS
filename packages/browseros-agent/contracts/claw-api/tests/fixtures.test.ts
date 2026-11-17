import { describe, expect, test } from 'bun:test'
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import type {
  CockpitStats,
  Plan,
  PlanKey,
  PlanKeyFullError,
} from '@browseros/claw-api'
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

interface ContractSchema {
  $ref?: string
  allOf?: ContractSchema[]
  nullable?: boolean
  type?: string
  enum?: unknown[]
  pattern?: string
  minLength?: number
  maxLength?: number
  minimum?: number
  required?: string[]
  properties?: Record<string, ContractSchema>
  items?: ContractSchema
}

type SchemaFile = Record<string, ContractSchema>

function schemaFiles(): Record<string, SchemaFile> {
  return {
    'plan.yaml': yaml('schemas/plan.yaml') as SchemaFile,
    'common.yaml': yaml('schemas/common.yaml') as SchemaFile,
  }
}

function resolveRef(
  ref: string,
  file: string,
  files: Record<string, SchemaFile>,
): [ContractSchema, string] {
  const [target, name] = ref.split('#/') as [string, string]
  const targetFile = target ? (target.split('/').at(-1) as string) : file
  const schema = files[targetFile]?.[name]
  if (!schema) throw new Error(`unresolved schema ref ${ref} in ${file}`)
  return [schema, targetFile]
}

interface ShapeContext {
  file: string
  files: Record<string, SchemaFile>
}

function shapeErrors(
  value: unknown,
  schema: ContractSchema,
  context: ShapeContext,
  path = '$',
): string[] {
  if (schema.$ref) {
    const [resolved, file] = resolveRef(
      schema.$ref,
      context.file,
      context.files,
    )
    return shapeErrors(value, resolved, { ...context, file }, path)
  }
  if (value === null) {
    return schema.nullable ? [] : [`${path} is null`]
  }
  if (schema.allOf) {
    return schema.allOf.flatMap((part) =>
      shapeErrors(value, part, context, path),
    )
  }
  if (schema.enum && !schema.enum.includes(value)) {
    return [`${path} is not one of ${schema.enum.join(', ')}`]
  }
  switch (schema.type) {
    case 'boolean':
      return typeof value === 'boolean' ? [] : [`${path} is not a boolean`]
    case 'integer':
      return Number.isSafeInteger(value) &&
        (value as number) >= (schema.minimum ?? Number.NEGATIVE_INFINITY)
        ? []
        : [`${path} is not an integer of at least ${schema.minimum}`]
    case 'string':
      return stringErrors(value, schema, path)
    case 'array':
      if (!Array.isArray(value)) return [`${path} is not an array`]
      return value.flatMap((item, index) =>
        shapeErrors(item, schema.items ?? {}, context, `${path}[${index}]`),
      )
    case 'object':
      return objectErrors(value, schema, context, path)
    default:
      return []
  }
}

function stringErrors(
  value: unknown,
  schema: ContractSchema,
  path: string,
): string[] {
  if (typeof value !== 'string') return [`${path} is not a string`]
  const fits =
    value.length >= (schema.minLength ?? 0) &&
    value.length <= (schema.maxLength ?? Number.POSITIVE_INFINITY) &&
    (!schema.pattern || new RegExp(schema.pattern).test(value))
  return fits ? [] : [`${path} does not fit its length or pattern`]
}

function objectErrors(
  value: unknown,
  schema: ContractSchema,
  context: ShapeContext,
  path: string,
): string[] {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return [`${path} is not an object`]
  }
  const record = value as Record<string, unknown>
  const properties = schema.properties ?? {}
  return [
    ...(schema.required ?? [])
      .filter((key) => !(key in record))
      .map((key) => `${path}.${key} is missing`),
    ...Object.entries(record).flatMap(([key, field]) => {
      const fieldSchema = properties[key]
      return fieldSchema
        ? shapeErrors(field, fieldSchema, context, `${path}.${key}`)
        : [`${path}.${key} is not in the schema`]
    }),
  ]
}

describe('plan API fixtures', () => {
  const planFixtures = {
    'plan-none.json': 'Plan',
    'plan-active.json': 'Plan',
    'plan-ending.json': 'Plan',
    'plan-payment-failed.json': 'Plan',
    'plan-unpaid.json': 'Plan',
    'plan-stale.json': 'Plan',
    'plan-unavailable.json': 'Plan',
    'plan-pricing.json': 'PlanPricing',
    'plan-checkout.json': 'PlanCheckout',
    'plan-checkout-complete.json': 'PlanCheckoutStatus',
    'plan-key.json': 'PlanKey',
    'plan-error-key-full.json': 'PlanKeyFullError',
    'plan-error-key-invalid.json': 'ApiError',
    'plan-error-already-subscribed.json': 'ApiError',
  } as const

  test('every plan fixture matches its schema', () => {
    const files = schemaFiles()
    for (const [name, model] of Object.entries(planFixtures)) {
      const file = model === 'ApiError' ? 'common.yaml' : 'plan.yaml'
      const schema = files[file]?.[model] ?? {}
      expect({
        name,
        errors: shapeErrors(fixture(name), schema, { file, files }),
      }).toEqual({ name, errors: [] })
    }
  })

  test('no fixture contains a licence key', () => {
    const licenceKey = /NEO(-[0-9A-Z]{5}){5}/
    for (const name of readdirSync(fixturesDir)) {
      expect({
        name,
        leaks: licenceKey.test(JSON.stringify(fixture(name))),
      }).toEqual({ name, leaks: false })
    }
  })

  test('the plan states carry the fields their status promises', () => {
    const unpaid = fixture('plan-unpaid.json') as Plan
    const ending = fixture('plan-ending.json') as Plan
    const failed = fixture('plan-payment-failed.json') as Plan
    const unavailable = fixture('plan-unavailable.json') as Plan
    const full = fixture('plan-error-key-full.json') as PlanKeyFullError

    expect(unpaid.features).toEqual([])
    expect(ending.endsAt).not.toBeNull()
    expect(failed.graceUntil).not.toBeNull()
    expect(unavailable.available).toBe(false)
    expect(full.code).toBe('key_full')
    expect(full.computers).toHaveLength(
      (fixture('plan-key.json') as PlanKey).cap,
    )
  })
})
