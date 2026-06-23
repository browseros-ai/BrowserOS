import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdir, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { basename, join } from 'node:path'
import { createGrepTool } from '../../../src/tools/filesystem/grep'
import {
  type FilesystemToolResult,
  MAX_GREP_FILE_SIZE,
} from '../../../src/tools/filesystem/utils'

let tmpDir: string
let exec: (params: Record<string, unknown>) => Promise<FilesystemToolResult>

beforeEach(async () => {
  tmpDir = join(
    tmpdir(),
    `fs-grep-test-${Date.now()}-${Math.random().toString(36).slice(2)}`,
  )
  await mkdir(tmpDir, { recursive: true })
  const tool = createGrepTool(tmpDir)
  // biome-ignore lint/suspicious/noExplicitAny: test helper
  exec = (params) => (tool as any).execute(params)
})

afterEach(async () => {
  await rm(tmpDir, { recursive: true, force: true })
})

async function createTestFiles() {
  await mkdir(join(tmpDir, 'src'), { recursive: true })
  await mkdir(join(tmpDir, 'lib'), { recursive: true })
  await writeFile(
    join(tmpDir, 'src', 'main.ts'),
    'import { foo } from "./utils"\nconst bar = foo()\nconsole.log(bar)',
  )
  await writeFile(
    join(tmpDir, 'src', 'utils.ts'),
    'export function foo() {\n  return 42\n}',
  )
  await writeFile(
    join(tmpDir, 'lib', 'helper.js'),
    'function helper() {\n  return "help"\n}',
  )
  await writeFile(
    join(tmpDir, 'readme.md'),
    '# Project\nThis is a project with foo and bar',
  )
}

describe('filesystem_grep', () => {
  it('finds matches across files', async () => {
    await createTestFiles()
    const result = await exec({ pattern: 'foo' })
    expect(result.isError).toBeUndefined()
    expect(result.text).toContain('src/main.ts')
    expect(result.text).toContain('src/utils.ts')
  })

  it('returns line numbers', async () => {
    await createTestFiles()
    const result = await exec({ pattern: 'foo' })
    expect(result.text).toMatch(/\d+/)
  })

  it('supports case-insensitive search', async () => {
    await writeFile(
      join(tmpDir, 'test.txt'),
      'Hello World\nhello world\nHELLO WORLD',
    )
    const result = await exec({ pattern: 'hello', ignore_case: true })
    expect(result.text).toContain('Hello World')
    expect(result.text).toContain('hello world')
    expect(result.text).toContain('HELLO WORLD')
  })

  it('supports literal string search', async () => {
    await writeFile(join(tmpDir, 'test.txt'), 'foo.bar()\nfooXbar()\nfoo bar')
    const result = await exec({ pattern: 'foo.bar()', literal: true })
    expect(result.text).toContain('foo.bar()')
    expect(result.text).not.toContain('fooXbar()')
  })

  it('supports glob filtering', async () => {
    await createTestFiles()
    const result = await exec({ pattern: 'function', glob: '*.ts' })
    expect(result.text).toContain('utils.ts')
    expect(result.text).not.toContain('helper.js')
  })

  it('supports context lines', async () => {
    await writeFile(
      join(tmpDir, 'ctx.txt'),
      'line 1\nline 2\nmatch here\nline 4\nline 5',
    )
    const result = await exec({ pattern: 'match here', context: 1 })
    expect(result.text).toContain('line 2')
    expect(result.text).toContain('match here')
    expect(result.text).toContain('line 4')
  })

  it('limits results', async () => {
    await writeFile(
      join(tmpDir, 'many.txt'),
      Array.from({ length: 200 }, (_, i) => `match line ${i}`).join('\n'),
    )
    const result = await exec({ pattern: 'match', limit: 5 })
    expect(result.text).toContain('limit=10')
  })

  it('returns no matches message', async () => {
    await writeFile(join(tmpDir, 'test.txt'), 'hello world')
    const result = await exec({ pattern: 'nonexistent_xyz' })
    expect(result.text).toContain('No matches')
  })

  it('handles invalid regex', async () => {
    const result = await exec({ pattern: '[invalid(' })
    expect(result.isError).toBe(true)
    expect(result.text).toContain('Invalid regex')
  })

  it('skips binary files', async () => {
    await writeFile(join(tmpDir, 'data.png'), 'fake png with searchterm')
    await writeFile(join(tmpDir, 'code.ts'), 'real code with searchterm')
    const result = await exec({ pattern: 'searchterm' })
    expect(result.text).toContain('code.ts')
    expect(result.text).not.toContain('data.png')
  })

  it('skips node_modules', async () => {
    await mkdir(join(tmpDir, 'node_modules', 'pkg'), { recursive: true })
    await writeFile(join(tmpDir, 'node_modules', 'pkg', 'index.js'), 'findme')
    await writeFile(join(tmpDir, 'src.ts'), 'findme')
    const result = await exec({ pattern: 'findme' })
    expect(result.text).toContain('src.ts')
    expect(result.text).not.toContain('node_modules')
  })

  it('searches a single file when path points to a file', async () => {
    await writeFile(join(tmpDir, 'single.txt'), 'alpha\nbeta\ngamma')
    const result = await exec({ pattern: 'beta', path: 'single.txt' })
    expect(result.text).toContain('beta')
    expect(result.text).not.toContain('alpha')
  })

  it('does not read oversized single-file targets', async () => {
    await writeFile(
      join(tmpDir, 'large.log'),
      'x'.repeat(MAX_GREP_FILE_SIZE + 1),
    )

    const result = await exec({ pattern: 'x', path: 'large.log' })

    expect(result.isError).toBe(true)
    expect(result.text).toContain('Unable to read file')
  })

  it('handles regex special characters', async () => {
    await writeFile(join(tmpDir, 'regex.txt'), 'price is $42.00\nno match')
    const result = await exec({ pattern: '\\$\\d+\\.\\d+' })
    expect(result.text).toContain('$42.00')
  })

  it('truncates long matching lines', async () => {
    const longLine = 'x'.repeat(1000)
    await writeFile(join(tmpDir, 'long.txt'), longLine)
    const result = await exec({ pattern: 'x' })
    expect(result.text).toContain('[truncated]')
  })

  it('rejects absolute search paths', async () => {
    const result = await exec({ pattern: 'anything', path: tmpDir })
    expect(result.isError).toBe(true)
    expect(result.text).toContain('relative to the selected workspace')
  })

  it('rejects traversal outside the workspace', async () => {
    const outsideDir = join(
      tmpdir(),
      `fs-grep-outside-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    )
    await mkdir(outsideDir, { recursive: true })
    try {
      const result = await exec({
        pattern: 'secret',
        path: `../${basename(outsideDir)}`,
      })
      expect(result.isError).toBe(true)
      expect(result.text).toContain('outside the selected workspace')
    } finally {
      await rm(outsideDir, { recursive: true, force: true })
    }
  })

  it('does not search symlinks that point outside the workspace', async () => {
    const outsideDir = join(
      tmpdir(),
      `fs-grep-symlink-outside-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    )
    await mkdir(outsideDir, { recursive: true })
    try {
      await writeFile(join(outsideDir, 'secret.txt'), 'secret-token')
      await symlink(join(outsideDir, 'secret.txt'), join(tmpDir, 'secret-link'))
      const result = await exec({ pattern: 'secret-token' })
      expect(result.isError).toBeUndefined()
      expect(result.text).toBe('No matches found.')
    } finally {
      await rm(outsideDir, { recursive: true, force: true })
    }
  })
})
