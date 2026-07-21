import { describe, expect, test } from 'bun:test'
import { readdir, readFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { parse } from 'yaml'

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
  '/api/v1/sessions/{sessionId}/browser-tabs/{browserTabId}/preview',
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

describe('TypeScript Claw API package boundary', () => {
  test('exports generated DTOs and wire enums without a transport client', async () => {
    const packageDirectory = resolve(import.meta.dir, '../../packages/claw-api')
    const generatedDirectory = join(packageDirectory, 'src/generated')
    const files = await treeFiles(generatedDirectory)
    const modelFiles = files.filter((file) => file.startsWith('models/'))

    expect(files).toContain('index.ts')
    expect(modelFiles.length).toBeGreaterThan(0)
    expect(
      files.every(
        (file) =>
          file === 'index.ts' || /^models\/[A-Za-z0-9]+\.ts$/.test(file),
      ),
    ).toBe(true)

    const generatedSource = (
      await Promise.all(
        files.map((file) => readFile(join(generatedDirectory, file), 'utf8')),
      )
    ).join('\n')
    expect(generatedSource).not.toMatch(
      /\b(?:Configuration|DefaultApi|ResponseError|FromJSON|ToJSON|runtime)\b/,
    )
    expect(await readFile(join(packageDirectory, 'src/index.ts'), 'utf8')).toBe(
      "export * from './generated/index.js'\n",
    )
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

async function treeFiles(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true })
  const nested = await Promise.all(
    entries.map(async (entry) => {
      if (!entry.isDirectory()) return [entry.name]
      return (await treeFiles(join(directory, entry.name))).map((file) =>
        join(entry.name, file),
      )
    }),
  )
  return nested.flat().toSorted()
}
