import { describe, expect, test } from 'bun:test'
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { basename, join, resolve } from 'node:path'

const sourceRoot = resolve(import.meta.dir, '../../src')

describe('claw-server API source layout', () => {
  test('keeps HTTP and MCP under the API boundary', () => {
    expect(existsSync(join(sourceRoot, 'routes'))).toBe(false)
    expect(existsSync(join(sourceRoot, 'mcp'))).toBe(false)
    expect(existsSync(join(sourceRoot, 'api/http'))).toBe(true)
    expect(existsSync(join(sourceRoot, 'api/mcp'))).toBe(true)
  })

  test('does not restore an API-wide production adapter', () => {
    const apiFiles = typescriptFiles(join(sourceRoot, 'api'))
    expect(
      apiFiles.filter((file) => basename(file) === 'production.ts'),
    ).toEqual([])

    const declarations = typescriptFiles(sourceRoot).filter((file) =>
      readFileSync(file, 'utf8').includes('CanonicalApiDependencies'),
    )
    expect(declarations).toEqual([])
  })

  test('keeps router construction and path registration out of HTTP handlers', () => {
    const handlers = typescriptFiles(join(sourceRoot, 'api/http/handlers'))
    const violations = handlers.flatMap((file) => {
      const source = readFileSync(file, 'utf8')
      return [
        /\bnew\s+Hono\b/.test(source) ? `${file}: Hono construction` : null,
        /\.(?:get|post|put|delete|patch|all)\s*\(/.test(source)
          ? `${file}: route registration`
          : null,
      ].filter((violation): violation is string => violation !== null)
    })
    expect(violations).toEqual([])
  })
})

function typescriptFiles(directory: string): string[] {
  if (!existsSync(directory)) return []
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name)
    if (entry.isDirectory()) return typescriptFiles(path)
    return entry.isFile() && entry.name.endsWith('.ts') ? [path] : []
  })
}
