import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdir, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { basename, join } from 'node:path'
import { createFindTool } from '../../../src/tools/filesystem/find'
import type { FilesystemToolResult } from '../../../src/tools/filesystem/utils'

let tmpDir: string
let exec: (params: Record<string, unknown>) => Promise<FilesystemToolResult>

beforeEach(async () => {
  tmpDir = join(
    tmpdir(),
    `fs-find-test-${Date.now()}-${Math.random().toString(36).slice(2)}`,
  )
  await mkdir(tmpDir, { recursive: true })
  const tool = createFindTool(tmpDir)
  // biome-ignore lint/suspicious/noExplicitAny: test helper
  exec = (params) => (tool as any).execute(params)
})

afterEach(async () => {
  await rm(tmpDir, { recursive: true, force: true })
})

async function createFileTree() {
  await mkdir(join(tmpDir, 'src', 'components'), { recursive: true })
  await mkdir(join(tmpDir, 'tests'), { recursive: true })
  await mkdir(join(tmpDir, 'docs'), { recursive: true })
  await writeFile(join(tmpDir, 'src', 'index.ts'), '')
  await writeFile(join(tmpDir, 'src', 'utils.ts'), '')
  await writeFile(join(tmpDir, 'src', 'components', 'button.tsx'), '')
  await writeFile(join(tmpDir, 'src', 'components', 'modal.tsx'), '')
  await writeFile(join(tmpDir, 'tests', 'index.test.ts'), '')
  await writeFile(join(tmpDir, 'docs', 'readme.md'), '')
  await writeFile(join(tmpDir, 'package.json'), '{}')
}

describe('filesystem_find', () => {
  it('finds files matching a glob pattern', async () => {
    await createFileTree()
    const result = await exec({ pattern: '*.ts' })
    expect(result.isError).toBeUndefined()
    expect(result.text).toContain('index.ts')
    expect(result.text).toContain('utils.ts')
    expect(result.text).toContain('index.test.ts')
  })

  it('finds tsx files', async () => {
    await createFileTree()
    const result = await exec({ pattern: '*.tsx' })
    expect(result.text).toContain('button.tsx')
    expect(result.text).toContain('modal.tsx')
    expect(result.text).not.toContain('.ts\n')
  })

  it('uses ** automatically for simple patterns', async () => {
    await createFileTree()
    const result = await exec({ pattern: '*.ts' })
    // Should find nested files too
    expect(result.text).toContain('src/index.ts')
    expect(result.text).toContain('tests/index.test.ts')
  })

  it('supports explicit recursive patterns', async () => {
    await createFileTree()
    const result = await exec({ pattern: 'src/**/*.ts' })
    expect(result.text).toContain('src/index.ts')
    expect(result.text).toContain('src/utils.ts')
    expect(result.text).not.toContain('tests/')
  })

  it('limits results', async () => {
    await createFileTree()
    const result = await exec({ pattern: '*', limit: 2 })
    const lines = result.text
      .split('\n')
      .filter((l) => l.trim() && !l.startsWith('('))
    expect(lines.length).toBe(2)
    expect(result.text).toContain('limit=4')
  })

  it('returns no matches message', async () => {
    await createFileTree()
    const result = await exec({ pattern: '*.xyz' })
    expect(result.text).toContain('No files matching')
  })

  it('searches a subdirectory', async () => {
    await createFileTree()
    const result = await exec({ pattern: '*.tsx', path: 'src/components' })
    expect(result.text).toContain('button.tsx')
    expect(result.text).toContain('modal.tsx')
  })

  it('skips node_modules', async () => {
    await mkdir(join(tmpDir, 'node_modules', 'pkg'), { recursive: true })
    await writeFile(join(tmpDir, 'node_modules', 'pkg', 'index.js'), '')
    await writeFile(join(tmpDir, 'src.js'), '')
    const result = await exec({ pattern: '*.js' })
    expect(result.text).toContain('src.js')
    expect(result.text).not.toContain('node_modules')
  })

  it('skips .git directory', async () => {
    await mkdir(join(tmpDir, '.git', 'objects'), { recursive: true })
    await writeFile(join(tmpDir, '.git', 'config'), '')
    await writeFile(join(tmpDir, 'real.txt'), '')
    const result = await exec({ pattern: '*' })
    expect(result.text).toContain('real.txt')
    expect(result.text).not.toContain('.git')
  })

  it('returns sorted results', async () => {
    await createFileTree()
    const result = await exec({ pattern: '*.ts' })
    const lines = result.text
      .split('\n')
      .filter((l) => l.trim() && !l.startsWith('('))
    const sorted = [...lines].sort()
    expect(lines).toEqual(sorted)
  })

  it('finds dotfiles', async () => {
    await writeFile(join(tmpDir, '.env'), 'SECRET=abc')
    await writeFile(join(tmpDir, '.gitignore'), 'node_modules')
    const result = await exec({ pattern: '.*' })
    expect(result.text).toContain('.env')
    expect(result.text).toContain('.gitignore')
  })

  it('finds JSON files', async () => {
    await createFileTree()
    const result = await exec({ pattern: '*.json' })
    expect(result.text).toContain('package.json')
  })

  it('rejects absolute search roots', async () => {
    const result = await exec({ pattern: '*', path: tmpDir })
    expect(result.isError).toBe(true)
    expect(result.text).toContain('relative to the selected workspace')
  })

  it('rejects traversal outside the workspace', async () => {
    const outsideDir = join(
      tmpdir(),
      `fs-find-outside-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    )
    await mkdir(outsideDir, { recursive: true })
    try {
      const result = await exec({
        pattern: '*',
        path: `../${basename(outsideDir)}`,
      })
      expect(result.isError).toBe(true)
      expect(result.text).toContain('outside the selected workspace')
    } finally {
      await rm(outsideDir, { recursive: true, force: true })
    }
  })

  it('skips symlinks that point outside the workspace', async () => {
    const outsideDir = join(
      tmpdir(),
      `fs-find-symlink-outside-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    )
    await mkdir(outsideDir, { recursive: true })
    try {
      await writeFile(join(outsideDir, 'secret.txt'), '')
      await symlink(outsideDir, join(tmpDir, 'outside-link'))
      const result = await exec({ pattern: '*' })
      expect(result.text).not.toContain('outside-link')
      expect(result.text).not.toContain('secret.txt')
    } finally {
      await rm(outsideDir, { recursive: true, force: true })
    }
  })
})
