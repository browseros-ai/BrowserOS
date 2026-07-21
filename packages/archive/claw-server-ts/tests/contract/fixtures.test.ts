/**
 * Validates every shared fixture against its canonical OpenAPI component.
 * The TypeScript DTO package intentionally has no runtime serializers; HTTP
 * behavior is covered by the cross-server suite, while this catches fixture
 * drift before the same values reach either generated model tree.
 */

import { describe, expect, test } from 'bun:test'
import { parse } from 'yaml'
import { canonicalApiError } from '../../src/lib/api-error'

const fixturesDirectory = new URL(
  '../../../../contracts/claw-api/fixtures/',
  import.meta.url,
)
const schemasDirectory = new URL(
  '../../../../contracts/claw-api/schemas/',
  import.meta.url,
)

interface OpenApiSchema {
  $ref?: string
  allOf?: OpenApiSchema[]
  type?: 'array' | 'boolean' | 'integer' | 'number' | 'object' | 'string'
  enum?: unknown[]
  format?: string
  minimum?: number
  additionalProperties?: boolean
  required?: string[]
  properties?: Record<string, OpenApiSchema>
  items?: OpenApiSchema
}

type SchemaDocument = Record<string, OpenApiSchema>
type SchemaDocuments = Record<string, SchemaDocument>

interface SchemaContext {
  documentName: string
  documents: SchemaDocuments
}

const schemaDocumentNames = [
  'common',
  'connections',
  'dispatches',
  'recordings',
  'sessions',
  'settings',
  'system',
] as const
const allSchemaDocuments = readSchemaDocuments()

const fixtures = [
  ['health.json', 'system', 'HealthResponse'],
  ['shutdown.json', 'system', 'ShutdownResponse'],
  ['system-info.json', 'system', 'SystemInfo'],
  ['telemetry-state.json', 'settings', 'TelemetryState'],
  ['session-list.json', 'sessions', 'SessionList'],
  ['session-detail.json', 'sessions', 'SessionDetail'],
  ['cancel-session.json', 'sessions', 'CancelSessionResponse'],
  ['recording-metadata.json', 'recordings', 'RecordingMetadata'],
  [
    'append-recording-events.json',
    'recordings',
    'AppendRecordingEventsResponse',
  ],
  ['connection.json', 'connections', 'Connection'],
  ['connection-list.json', 'connections', 'ConnectionList'],
  ['api-error.json', 'common', 'ApiError'],
  ['api-error-minimal.json', 'common', 'ApiError'],
] as const

describe('canonical contract fixtures', () => {
  for (const [file, documentName, schemaName] of fixtures) {
    test(`validates ${file} against ${schemaName}`, async () => {
      const [fixture, documents] = await Promise.all([
        Bun.file(new URL(file, fixturesDirectory)).json(),
        allSchemaDocuments,
      ])
      const schemas = documents[documentName] ?? {}
      const schema = schemas[schemaName]

      expect(schema).toBeDefined()
      expect(
        validateSchema(fixture, schema ?? {}, { documentName, documents }),
      ).toEqual([])
    })
  }

  test('rejects undeclared fields and invalid enum values', async () => {
    const documents = await allSchemaDocuments
    const schemas = documents.system ?? {}
    const errors = validateSchema(
      { status: 'broken', extra: true },
      schemas.HealthResponse ?? {},
      { documentName: 'system', documents },
    )

    expect(errors).toContain('$.extra is not declared by the schema')
    expect(errors).toContain('$.status is not an allowed enum value')
  })

  test('rejects strings that violate URI formats', async () => {
    const documents = await allSchemaDocuments
    const schemas = documents.system ?? {}
    const errors = validateSchema(
      { product: 'BrowserClaw', version: '1.0.0', url: 'not a URI' },
      schemas.SystemInfo ?? {},
      { documentName: 'system', documents },
    )

    expect(errors).toContain('$.url must be a URI')
  })

  test('canonical errors omit an unavailable request id', () => {
    expect(canonicalApiError('not_found', 'Missing')).toEqual({
      code: 'not_found',
      message: 'Missing',
    })
    expect(canonicalApiError('not_found', 'Missing', 'request-1')).toEqual({
      code: 'not_found',
      message: 'Missing',
      requestId: 'request-1',
    })
  })
})

async function readSchemaDocument(name: string): Promise<SchemaDocument> {
  return parse(await Bun.file(new URL(`${name}.yaml`, schemasDirectory)).text())
}

async function readSchemaDocuments(): Promise<SchemaDocuments> {
  return Object.fromEntries(
    await Promise.all(
      schemaDocumentNames.map(async (name) => [
        name,
        await readSchemaDocument(name),
      ]),
    ),
  )
}

function validateSchema(
  value: unknown,
  schema: OpenApiSchema,
  context: SchemaContext,
  path = '$',
): string[] {
  if (schema.$ref) {
    const match = schema.$ref.match(
      /^(?:\.\/([a-z][a-z0-9_]*)\.yaml)?#\/([A-Za-z][A-Za-z0-9]*)$/,
    )
    const documentName = match?.[1] ?? context.documentName
    const target = match
      ? context.documents[documentName]?.[match[2] as string]
      : undefined
    return target
      ? validateSchema(value, target, { ...context, documentName }, path)
      : [`${path} references unknown schema ${schema.$ref}`]
  }

  const errors = (schema.allOf ?? []).flatMap((part) =>
    validateSchema(value, part, context, path),
  )
  if (
    schema.enum &&
    !schema.enum.some((candidate) => Object.is(candidate, value))
  ) {
    errors.push(`${path} is not an allowed enum value`)
  }

  errors.push(...validateTypedValue(value, schema, context, path))
  if (schema.format === 'uri' && !isUri(value)) {
    errors.push(`${path} must be a URI`)
  }
  if (
    schema.minimum !== undefined &&
    typeof value === 'number' &&
    value < schema.minimum
  ) {
    errors.push(`${path} must be at least ${schema.minimum.toString()}`)
  }
  return errors
}

function isUri(value: unknown): boolean {
  if (typeof value !== 'string') return false
  try {
    new URL(value)
    return true
  } catch {
    return false
  }
}

function validateTypedValue(
  value: unknown,
  schema: OpenApiSchema,
  context: SchemaContext,
  path: string,
): string[] {
  switch (schema.type) {
    case 'object':
      return validateObject(value, schema, context, path)
    case 'array':
      return validateArray(value, schema.items, context, path)
    case 'integer':
      return Number.isInteger(value) ? [] : [`${path} must be an integer`]
    case 'number':
      return typeof value === 'number' && Number.isFinite(value)
        ? []
        : [`${path} must be a finite number`]
    case 'string':
      return typeof value === 'string' ? [] : [`${path} must be a string`]
    case 'boolean':
      return typeof value === 'boolean' ? [] : [`${path} must be a boolean`]
    default:
      return []
  }
}

function validateObject(
  value: unknown,
  schema: OpenApiSchema,
  context: SchemaContext,
  path: string,
): string[] {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return [`${path} must be an object`]
  }
  const errors: string[] = []
  const record = value as Record<string, unknown>
  const properties = schema.properties ?? {}
  for (const required of schema.required ?? []) {
    if (!Object.hasOwn(record, required)) {
      errors.push(`${path}.${required} is required`)
    }
  }
  if (schema.additionalProperties === false) {
    for (const key of Object.keys(record)) {
      if (!Object.hasOwn(properties, key)) {
        errors.push(`${path}.${key} is not declared by the schema`)
      }
    }
  }
  for (const [key, propertySchema] of Object.entries(properties)) {
    if (Object.hasOwn(record, key)) {
      errors.push(
        ...validateSchema(
          record[key],
          propertySchema,
          context,
          `${path}.${key}`,
        ),
      )
    }
  }
  return errors
}

function validateArray(
  value: unknown,
  items: OpenApiSchema | undefined,
  context: SchemaContext,
  path: string,
): string[] {
  if (!Array.isArray(value)) return [`${path} must be an array`]
  if (!items) return []
  return value.flatMap((item, index) =>
    validateSchema(item, items, context, `${path}[${index}]`),
  )
}
