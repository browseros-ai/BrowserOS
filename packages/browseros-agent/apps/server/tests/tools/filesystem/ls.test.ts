import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdir, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { basename, join } from 'node:path'
import { createLsTool } from '../../../src/tools/filesystem/ls'
import type { FilesystemToolResult } from '../../../src/tools/filesystem/utils'

let tmpDir: string
let exec: (params: Record<string, unknown>) => Promise<FilesystemToolResult>

beforeEach(async () => {
  tmpDir = join(
    tmpdir(),
    `fs-ls-test-${Date.now()}-${Math.random().toString(36).slice(2)}`,
  )
  await mkdir(tmpDir, { recursive: true })
  const tool = createLsTool(tmpDir)
  // biome-ignore lint/suspicious/noExplicitAny: test helper
  exec = (params) => (tool as any).execute(params)
})

afterEach(async () => {
  await rm(tmpDir, { recursive: true, force: true })
})

describe('filesystem_ls', () => {
  it('lists files and directories', async () => {
    await mkdir(join(tmpDir, 'src'))
    await writeFile(join(tmpDir, 'package.json'), '{}')
    await writeFile(join(tmpDir, 'index.ts'), 'export {}')

    const result = await exec({})
    expect(result.isError).toBeUndefined()
    expect(result.text).toContain('src/')
    expect(result.text).toContain('package.json')
    expect(result.text).toContain('index.ts')
  })

  it('shows directories first', async () => {
    await mkdir(join(tmpDir, 'aaa-dir'))
    await writeFile(join(tmpDir, 'bbb-file.txt'), 'x')

    const result = await exec({})
    const lines = result.text.split('\n')
    const dirIdx = lines.findIndex((l) => l.includes('aaa-dir/'))
    const fileIdx = lines.findIndex((l) => l.includes('bbb-file.txt'))
    expect(dirIdx).toBeLessThan(fileIdx)
  })

  it('shows file sizes', async () => {
    await writeFile(join(tmpDir, 'small.txt'), 'hello')
    const result = await exec({})
    expect(result.text).toMatch(/small\.txt \(\d+B\)/)
  })

  it('handles empty directory', async () => {
    const result = await exec({})
    expect(result.text).toBe('(empty directory)')
  })

  it('lists a subdirectory', async () => {
    await mkdir(join(tmpDir, 'sub'))
    await writeFile(join(tmpDir, 'sub', 'file.txt'), 'content')
    const result = await exec({ path: 'sub' })
    expect(result.text).toContain('file.txt')
  })

  it('limits entries', async () => {
    for (let i = 0; i < 10; i++) {
      await writeFile(join(tmpDir, `file${i}.txt`), `content ${i}`)
    }
    const result = await exec({ limit: 3 })
    const lines = result.text
      .split('\n')
      .filter((l) => l.trim() && !l.startsWith('('))
    expect(lines.length).toBe(3)
    expect(result.text).toContain('limit=6')
  })

  it('sorts entries alphabetically', async () => {
    await writeFile(join(tmpDir, 'charlie.txt'), '')
    await writeFile(join(tmpDir, 'alpha.txt'), '')
    await writeFile(join(tmpDir, 'bravo.txt'), '')

    const result = await exec({})
    const lines = result.text.split('\n').filter(Boolean)
    const names = lines.map((l) => l.split(' ')[0])
    expect(names).toEqual([...names].sort())
  })

  it('errors for nonexistent directory', async () => {
    const result = await exec({ path: 'nonexistent' })
    expect(result.isError).toBe(true)
  })

  it('handles large files size formatting', async () => {
    const largeContent = 'x'.repeat(2 * 1024 * 1024)
    await writeFile(join(tmpDir, 'large.bin'), largeContent)
    const result = await exec({})
    expect(result.text).toContain('MB')
  })

  it('rejects absolute paths', async () => {
    await writeFile(join(tmpDir, 'abs.txt'), 'data')
    const result = await exec({ path: tmpDir })
    expect(result.isError).toBe(true)
    expect(result.text).toContain('relative to the selected workspace')
  })

  it('rejects traversal outside the workspace', async () => {
    const outsideDir = join(
      tmpdir(),
      `fs-ls-outside-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    )
    await mkdir(outsideDir, { recursive: true })
    try {
      const result = await exec({ path: `../${basename(outsideDir)}` })
      expect(result.isError).toBe(true)
      expect(result.text).toContain('outside the selected workspace')
    } finally {
      await rm(outsideDir, { recursive: true, force: true })
    }
  })

  it('rejects symlinked directories outside the workspace', async () => {
    const outsideDir = join(
      tmpdir(),
      `fs-ls-symlink-outside-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    )
    await mkdir(outsideDir, { recursive: true })
    try {
      await symlink(outsideDir, join(tmpDir, 'outside-link'))
      const result = await exec({ path: 'outside-link' })
      expect(result.isError).toBe(true)
      expect(result.text).toContain('outside the selected workspace')
    } finally {
      await rm(outsideDir, { recursive: true, force: true })
    }
  })
})
