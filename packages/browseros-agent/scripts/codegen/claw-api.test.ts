import { describe, expect, test } from 'bun:test'
import { readdir, readFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { parse } from 'yaml'
import { flattenRequiredHeaderGuards } from './claw-api'

interface OpenApiOperation {
  operationId?: string
  parameters?: Array<{
    in?: string
    name?: string
  }>
}

interface OpenApiDocument {
  openapi?: string
  paths?: Record<string, Record<string, OpenApiOperation> & { $ref?: string }>
  components?: {
    schemas?: Record<string, unknown>
  }
}

const contractPath = join(
  import.meta.dir,
  '../../contracts/claw-api/openapi.yaml',
)
const contractDirectory = dirname(contractPath)

const expectedPaths = [
  '/system/health',
  '/system/shutdown',
  '/api/v1/system',
  '/api/v1/settings/telemetry',
  '/api/v1/recordings/events',
  '/api/v1/sessions',
  '/api/v1/sessions/{sessionId}',
  '/api/v1/sessions/{sessionId}/cancel',
  '/api/v1/sessions/{sessionId}/recording',
  '/api/v1/sessions/{sessionId}/recording/events',
  '/api/v1/tabs',
  '/api/v1/tabs/{pageId}/preview',
  '/api/v1/dispatches/{dispatchId}/screenshot',
  '/api/v1/connections',
  '/api/v1/connections/{harness}',
]

describe('BrowserClaw OpenAPI contract', () => {
  test('defines only the approved canonical surface with unique operation IDs', async () => {
    const source = await readFile(contractPath, 'utf8')
    const document = parse(source) as OpenApiDocument

    expect(document.openapi).toBe('3.0.3')
    expect(Object.keys(document.paths ?? {}).sort()).toEqual(
      expectedPaths.toSorted(),
    )

    const pathItems = await Promise.all(
      Object.values(document.paths ?? {}).map(async (path) => {
        if (!path.$ref) return path
        return parse(
          await readFile(resolve(contractDirectory, path.$ref), 'utf8'),
        ) as Record<string, OpenApiOperation>
      }),
    )
    const operations = pathItems.flatMap((path) =>
      Object.entries(path).filter(([method]) =>
        ['get', 'put', 'post', 'delete', 'patch'].includes(method),
      ),
    )
    const operationIds = operations.map(
      ([, operation]) => operation.operationId,
    )
    expect(operationIds.every(Boolean)).toBe(true)
    expect(new Set(operationIds).size).toBe(operationIds.length)
  })

  test('does not expose legacy execution identities', async () => {
    const sources = await Promise.all(
      (await yamlFiles(contractDirectory)).map((path) =>
        readFile(path, 'utf8'),
      ),
    )
    expect(sources.join('\n')).not.toMatch(/\b(?:agentId|taskId|runId)\b/)
  })
})

describe('flattenRequiredHeaderGuards', () => {
  test('flattens required header assignments and preserves optional guards', () => {
    const generated = `        if (requestParameters['required'] == null) {
            throw new Error('required');
        }

        if (requestParameters['required'] != null) {
            headerParameters['Required'] = String(requestParameters['required']);
        }

        if (requestParameters['optional'] != null) {
            headerParameters['Optional'] = String(requestParameters['optional']);
        }
`

    expect(
      flattenRequiredHeaderGuards(generated),
    ).toBe(`        if (requestParameters['required'] == null) {
            throw new Error('required');
        }

        headerParameters['Required'] = String(requestParameters['required']);

        if (requestParameters['optional'] != null) {
            headerParameters['Optional'] = String(requestParameters['optional']);
        }
`)
  })
})

async function yamlFiles(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true })
  const nested = await Promise.all(
    entries.map((entry) => {
      const path = join(directory, entry.name)
      if (entry.isDirectory()) return yamlFiles(path)
      return Promise.resolve(entry.name.endsWith('.yaml') ? [path] : [])
    }),
  )
  return nested.flat()
}
