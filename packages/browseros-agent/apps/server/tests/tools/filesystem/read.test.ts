import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdir, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createReadTool } from '../../../src/tools/filesystem/read'
import type { FilesystemToolResult } from '../../../src/tools/filesystem/utils'
import {
  MAX_READ_CHARS,
  MAX_READ_LINES,
  WORKSPACE_PATH_ERROR,
} from '../../../src/tools/filesystem/utils'

let tmpDir: string
let outsideDir: string
let exec: (params: Record<string, unknown>) => Promise<FilesystemToolResult>

beforeEach(async () => {
  const suffix = `${Date.now()}-${Math.random().toString(36).slice(2)}`
  tmpDir = join(tmpdir(), `fs-read-test-${suffix}`)
  outsideDir = join(tmpdir(), `fs-read-outside-${suffix}`)
  await mkdir(tmpDir, { recursive: true })
  await mkdir(outsideDir, { recursive: true })
  const tool = createReadTool(tmpDir)
  // biome-ignore lint/suspicious/noExplicitAny: test helper
  exec = (params) => (tool as any).execute(params)
})

afterEach(async () => {
  await rm(tmpDir, { recursive: true, force: true })
  await rm(outsideDir, { recursive: true, force: true })
})

describe('filesystem_read', () => {
  it('reads a text file with line numbers', async () => {
    await writeFile(join(tmpDir, 'hello.txt'), 'line one\nline two\nline three')
    const result = await exec({ path: 'hello.txt' })
    expect(result.isError).toBeUndefined()
    expect(result.text).toContain('1 | line one')
    expect(result.text).toContain('2 | line two')
    expect(result.text).toContain('3 | line three')
  })

  it('reads with offset', async () => {
    await writeFile(join(tmpDir, 'lines.txt'), 'a\nb\nc\nd\ne')
    const result = await exec({ path: 'lines.txt', offset: 3 })
    expect(result.text).toContain('3 | c')
    expect(result.text).toContain('4 | d')
    expect(result.text).not.toContain('1 | a')
  })

  it('reads with limit', async () => {
    await writeFile(join(tmpDir, 'lines.txt'), 'a\nb\nc\nd\ne')
    const result = await exec({ path: 'lines.txt', limit: 2 })
    expect(result.text).toContain('1 | a')
    expect(result.text).toContain('2 | b')
    expect(result.text).not.toContain('3 | c')
  })

  it('reads with offset and limit', async () => {
    await writeFile(join(tmpDir, 'lines.txt'), 'a\nb\nc\nd\ne')
    const result = await exec({ path: 'lines.txt', offset: 2, limit: 2 })
    expect(result.text).toContain('2 | b')
    expect(result.text).toContain('3 | c')
    expect(result.text).not.toContain('1 | a')
    expect(result.text).not.toContain('4 | d')
  })

  it('handles offset beyond end of file', async () => {
    await writeFile(join(tmpDir, 'short.txt'), 'a\nb')
    const result = await exec({ path: 'short.txt', offset: 100 })
    expect(result.text).toContain('2 lines')
    expect(result.text).toContain('beyond end')
  })

  it('reads an empty file', async () => {
    await writeFile(join(tmpDir, 'empty.txt'), '')
    const result = await exec({ path: 'empty.txt' })
    expect(result.isError).toBeUndefined()
    expect(result.text).toContain('1 | ')
  })

  it('reads an image file and returns base64', async () => {
    const pngHeader = Buffer.from([
      0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
    ])
    await writeFile(join(tmpDir, 'image.png'), pngHeader)
    const result = await exec({ path: 'image.png' })
    expect(result.images).toBeDefined()
    expect(result.images?.length).toBe(1)
    expect(result.images?.[0].mimeType).toBe('image/png')
    expect(result.images?.[0].data).toBe(pngHeader.toString('base64'))
    expect(result.text).toContain('Image:')
  })

  it('returns error for nonexistent file', async () => {
    const result = await exec({ path: 'nonexistent.txt' })
    expect(result.isError).toBe(true)
  })

  it('resolves relative paths against cwd', async () => {
    await mkdir(join(tmpDir, 'sub'), { recursive: true })
    await writeFile(join(tmpDir, 'sub', 'nested.txt'), 'nested content')
    const result = await exec({ path: 'sub/nested.txt' })
    expect(result.text).toContain('nested content')
  })

  it('handles absolute paths inside the workspace', async () => {
    const absPath = join(tmpDir, 'abs.txt')
    await writeFile(absPath, 'absolute')
    const result = await exec({ path: absPath })
    expect(result.text).toContain('absolute')
  })

  it('rejects parent traversal outside the workspace', async () => {
    const result = await exec({ path: '../secret.txt' })
    expect(result.isError).toBe(true)
    expect(result.text).toContain(WORKSPACE_PATH_ERROR)
  })

  it('rejects absolute paths outside the workspace', async () => {
    const outsideFile = join(outsideDir, 'secret.txt')
    await writeFile(outsideFile, 'secret')

    const result = await exec({ path: outsideFile })

    expect(result.isError).toBe(true)
    expect(result.text).toContain(WORKSPACE_PATH_ERROR)
  })

  it('rejects symlinked files outside the workspace', async () => {
    const outsideFile = join(outsideDir, 'secret.txt')
    await writeFile(outsideFile, 'secret')
    await symlink(outsideFile, join(tmpDir, 'secret-link.txt'))

    const result = await exec({ path: 'secret-link.txt' })

    expect(result.isError).toBe(true)
    expect(result.text).toContain(WORKSPACE_PATH_ERROR)
  })

  it('errors when a read would exceed the line limit', async () => {
    const manyLines = Array.from(
      { length: MAX_READ_LINES + 50 },
      (_, i) => `line ${i + 1}`,
    ).join('\n')
    await writeFile(join(tmpDir, 'large.txt'), manyLines)
    const result = await exec({ path: 'large.txt' })
    expect(result.isError).toBe(true)
    expect(result.text).toContain(`${MAX_READ_LINES}-line limit`)
  })

  it('errors when the requested limit exceeds the maximum allowed lines', async () => {
    const manyLines = Array.from(
      { length: 50 },
      (_, i) => `line ${i + 1}`,
    ).join('\n')
    await writeFile(join(tmpDir, 'limited.txt'), manyLines)
    const result = await exec({
      path: 'limited.txt',
      limit: MAX_READ_LINES + 1,
    })
    expect(result.isError).toBe(true)
    expect(result.text).toContain(`at most ${MAX_READ_LINES} lines`)
  })

  it('errors when limit is zero', async () => {
    await writeFile(join(tmpDir, 'zero.txt'), 'a\nb\nc')
    const result = await exec({ path: 'zero.txt', limit: 0 })
    expect(result.isError).toBe(true)
    expect(result.text).toContain('greater than 0')
  })

  it('errors when a requested range exceeds the character limit', async () => {
    const longLine = 'x'.repeat(MAX_READ_CHARS + 100)
    await writeFile(join(tmpDir, 'chars.txt'), longLine)
    const result = await exec({ path: 'chars.txt', limit: 1 })
    expect(result.isError).toBe(true)
    expect(result.text).toContain(`${MAX_READ_CHARS}-character limit`)
  })

  it('handles files with UTF-8 BOM', async () => {
    await writeFile(join(tmpDir, 'bom.txt'), '\uFEFFhello bom')
    const result = await exec({ path: 'bom.txt' })
    expect(result.text).toContain('hello bom')
  })

  it('handles various image extensions', async () => {
    const exts = ['.jpg', '.jpeg', '.gif', '.webp', '.svg']
    for (const ext of exts) {
      await writeFile(join(tmpDir, `img${ext}`), 'fake image data')
      const result = await exec({ path: `img${ext}` })
      expect(result.images).toBeDefined()
      expect(result.images?.length).toBe(1)
    }
  })
})
