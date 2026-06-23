import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdir, readFile, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { basename, join } from 'node:path'
import { createEditTool } from '../../../src/tools/filesystem/edit'
import type { FilesystemToolResult } from '../../../src/tools/filesystem/utils'

let tmpDir: string
let exec: (params: Record<string, unknown>) => Promise<FilesystemToolResult>

beforeEach(async () => {
  tmpDir = join(
    tmpdir(),
    `fs-edit-test-${Date.now()}-${Math.random().toString(36).slice(2)}`,
  )
  await mkdir(tmpDir, { recursive: true })
  const tool = createEditTool(tmpDir)
  // biome-ignore lint/suspicious/noExplicitAny: test helper
  exec = (params) => (tool as any).execute(params)
})

afterEach(async () => {
  await rm(tmpDir, { recursive: true, force: true })
})

describe('filesystem_edit', () => {
  it('replaces an exact match', async () => {
    await writeFile(
      join(tmpDir, 'test.ts'),
      'const x = 1\nconst y = 2\nconst z = 3',
    )
    const result = await exec({
      path: 'test.ts',
      old_string: 'const y = 2',
      new_string: 'const y = 42',
    })
    expect(result.isError).toBeUndefined()
    expect(result.text).toContain('Applied edit')

    const content = await readFile(join(tmpDir, 'test.ts'), 'utf-8')
    expect(content).toBe('const x = 1\nconst y = 42\nconst z = 3')
  })

  it('errors when old_string not found', async () => {
    await writeFile(join(tmpDir, 'test.ts'), 'const x = 1')
    const result = await exec({
      path: 'test.ts',
      old_string: 'nonexistent',
      new_string: 'replacement',
    })
    expect(result.isError).toBe(true)
    expect(result.text).toContain('not found')
  })

  it('errors when multiple occurrences found', async () => {
    await writeFile(join(tmpDir, 'test.ts'), 'foo\nbar\nfoo\nbaz')
    const result = await exec({
      path: 'test.ts',
      old_string: 'foo',
      new_string: 'qux',
    })
    expect(result.isError).toBe(true)
    expect(result.text).toContain('2')
    expect(result.text).toContain('occurrences')
  })

  it('errors when old_string equals new_string', async () => {
    await writeFile(join(tmpDir, 'test.ts'), 'const x = 1')
    const result = await exec({
      path: 'test.ts',
      old_string: 'const x = 1',
      new_string: 'const x = 1',
    })
    expect(result.isError).toBe(true)
    expect(result.text).toContain('identical')
  })

  it('errors for nonexistent file', async () => {
    const result = await exec({
      path: 'missing.ts',
      old_string: 'x',
      new_string: 'y',
    })
    expect(result.isError).toBe(true)
  })

  it('performs fuzzy match when whitespace differs', async () => {
    await writeFile(
      join(tmpDir, 'test.ts'),
      '  const x = 1\n  const y = 2\n  const z = 3',
    )
    const result = await exec({
      path: 'test.ts',
      old_string: 'const y = 2',
      new_string: '  const y = 42',
    })
    expect(result.isError).toBeUndefined()
    expect(result.text).toContain('Applied edit')

    const content = await readFile(join(tmpDir, 'test.ts'), 'utf-8')
    expect(content).toContain('const y = 42')
  })

  it('preserves CRLF line endings', async () => {
    await writeFile(join(tmpDir, 'crlf.ts'), 'line 1\r\nline 2\r\nline 3')
    await exec({
      path: 'crlf.ts',
      old_string: 'line 2',
      new_string: 'modified line 2',
    })

    const content = await readFile(join(tmpDir, 'crlf.ts'), 'utf-8')
    expect(content).toContain('\r\n')
    expect(content).toContain('modified line 2')
  })

  it('preserves UTF-8 BOM', async () => {
    await writeFile(join(tmpDir, 'bom.ts'), '\uFEFFconst x = 1')
    await exec({
      path: 'bom.ts',
      old_string: 'const x = 1',
      new_string: 'const x = 2',
    })

    const content = await readFile(join(tmpDir, 'bom.ts'), 'utf-8')
    expect(content.charCodeAt(0)).toBe(0xfeff)
    expect(content).toContain('const x = 2')
  })

  it('generates a diff in the output', async () => {
    await writeFile(join(tmpDir, 'test.ts'), 'hello world')
    const result = await exec({
      path: 'test.ts',
      old_string: 'hello world',
      new_string: 'goodbye world',
    })
    expect(result.text).toContain('- hello world')
    expect(result.text).toContain('+ goodbye world')
  })

  it('handles multiline replacements', async () => {
    await writeFile(
      join(tmpDir, 'multi.ts'),
      'function foo() {\n  return 1\n}\n\nfunction bar() {\n  return 2\n}',
    )
    const result = await exec({
      path: 'multi.ts',
      old_string: 'function foo() {\n  return 1\n}',
      new_string: 'function foo() {\n  return 42\n  // updated\n}',
    })
    expect(result.isError).toBeUndefined()

    const content = await readFile(join(tmpDir, 'multi.ts'), 'utf-8')
    expect(content).toContain('return 42')
    expect(content).toContain('// updated')
    expect(content).toContain('function bar')
  })

  it('handles replacement that changes line count', async () => {
    await writeFile(join(tmpDir, 'grow.ts'), 'a\nb\nc')
    await exec({
      path: 'grow.ts',
      old_string: 'b',
      new_string: 'b1\nb2\nb3',
    })

    const content = await readFile(join(tmpDir, 'grow.ts'), 'utf-8')
    expect(content).toBe('a\nb1\nb2\nb3\nc')
  })

  it('handles replacement that removes lines', async () => {
    await writeFile(join(tmpDir, 'shrink.ts'), 'a\nb\nc\nd\ne')
    await exec({
      path: 'shrink.ts',
      old_string: 'b\nc\nd',
      new_string: 'replaced',
    })

    const content = await readFile(join(tmpDir, 'shrink.ts'), 'utf-8')
    expect(content).toBe('a\nreplaced\ne')
  })

  it('rejects absolute paths', async () => {
    const absPath = join(tmpDir, 'abs.ts')
    await writeFile(absPath, 'const x = 1')
    const result = await exec({
      path: absPath,
      old_string: 'const x = 1',
      new_string: 'const x = 2',
    })
    expect(result.isError).toBe(true)
    expect(result.text).toContain('relative to the selected workspace')
  })

  it('rejects traversal outside the workspace', async () => {
    const outsideDir = join(
      tmpdir(),
      `fs-edit-outside-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    )
    await mkdir(outsideDir, { recursive: true })
    try {
      await writeFile(join(outsideDir, 'secret.ts'), 'const secret = true')
      const result = await exec({
        path: `../${basename(outsideDir)}/secret.ts`,
        old_string: 'true',
        new_string: 'false',
      })
      expect(result.isError).toBe(true)
      expect(result.text).toContain('outside the selected workspace')
    } finally {
      await rm(outsideDir, { recursive: true, force: true })
    }
  })

  it('rejects symlinks that point outside the workspace', async () => {
    const outsideDir = join(
      tmpdir(),
      `fs-edit-symlink-outside-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    )
    await mkdir(outsideDir, { recursive: true })
    try {
      await writeFile(join(outsideDir, 'secret.ts'), 'const secret = true')
      await symlink(join(outsideDir, 'secret.ts'), join(tmpDir, 'secret-link'))
      const result = await exec({
        path: 'secret-link',
        old_string: 'true',
        new_string: 'false',
      })
      expect(result.isError).toBe(true)
      expect(result.text).toContain('outside the selected workspace')
    } finally {
      await rm(outsideDir, { recursive: true, force: true })
    }
  })
})
