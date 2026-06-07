import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdir, realpath, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  detectLineEnding,
  isBinaryPath,
  normalizeToLF,
  resolveWorkspacePath,
  resolveWorkspaceRoot,
  restoreLineEndings,
  stripBom,
  truncateHead,
  truncateLine,
  truncateTail,
  WORKSPACE_PATH_ERROR,
  walkFiles,
} from '../../../src/tools/filesystem/utils'

describe('truncateHead', () => {
  it('does not truncate small content', () => {
    const result = truncateHead('a\nb\nc')
    expect(result.truncated).toBe(false)
    expect(result.content).toBe('a\nb\nc')
    expect(result.totalLines).toBe(3)
    expect(result.keptLines).toBe(3)
  })

  it('truncates by line count', () => {
    const content = Array.from({ length: 100 }, (_, i) => `line ${i}`).join(
      '\n',
    )
    const result = truncateHead(content, 10)
    expect(result.truncated).toBe(true)
    expect(result.keptLines).toBe(10)
    expect(result.totalLines).toBe(100)
  })

  it('truncates by byte size', () => {
    const content = Array.from({ length: 100 }, () => 'x'.repeat(200)).join(
      '\n',
    )
    const result = truncateHead(content, 10000, 1024)
    expect(result.truncated).toBe(true)
    expect(Buffer.byteLength(result.content, 'utf-8')).toBeLessThanOrEqual(
      1024 + 200,
    )
  })

  it('handles empty content', () => {
    const result = truncateHead('')
    expect(result.truncated).toBe(false)
    expect(result.content).toBe('')
  })
})

describe('truncateTail', () => {
  it('does not truncate small content', () => {
    const result = truncateTail('a\nb\nc')
    expect(result.truncated).toBe(false)
    expect(result.content).toBe('a\nb\nc')
  })

  it('keeps last N lines', () => {
    const lines = Array.from({ length: 100 }, (_, i) => `line ${i}`)
    const content = lines.join('\n')
    const result = truncateTail(content, 10)
    expect(result.truncated).toBe(true)
    expect(result.keptLines).toBe(10)
    expect(result.content).toContain('line 99')
    expect(result.content).not.toContain('line 0')
  })

  it('handles empty content', () => {
    const result = truncateTail('')
    expect(result.truncated).toBe(false)
    expect(result.content).toBe('')
  })
})

describe('truncateLine', () => {
  it('does not truncate short lines', () => {
    expect(truncateLine('hello', 500)).toBe('hello')
  })

  it('truncates long lines', () => {
    const long = 'x'.repeat(600)
    const result = truncateLine(long, 500)
    expect(result.length).toBe(500 + ' [truncated]'.length)
    expect(result).toContain('[truncated]')
  })
})

describe('stripBom', () => {
  it('strips BOM', () => {
    const result = stripBom('\uFEFFhello')
    expect(result.hasBom).toBe(true)
    expect(result.content).toBe('hello')
  })

  it('handles content without BOM', () => {
    const result = stripBom('hello')
    expect(result.hasBom).toBe(false)
    expect(result.content).toBe('hello')
  })
})

describe('detectLineEnding', () => {
  it('detects CRLF', () => {
    expect(detectLineEnding('a\r\nb')).toBe('\r\n')
  })

  it('detects CR', () => {
    expect(detectLineEnding('a\rb')).toBe('\r')
  })

  it('defaults to LF', () => {
    expect(detectLineEnding('a\nb')).toBe('\n')
    expect(detectLineEnding('abc')).toBe('\n')
  })
})

describe('normalizeToLF', () => {
  it('converts CRLF to LF', () => {
    expect(normalizeToLF('a\r\nb')).toBe('a\nb')
  })

  it('converts CR to LF', () => {
    expect(normalizeToLF('a\rb')).toBe('a\nb')
  })

  it('preserves LF', () => {
    expect(normalizeToLF('a\nb')).toBe('a\nb')
  })
})

describe('restoreLineEndings', () => {
  it('converts LF to CRLF', () => {
    expect(restoreLineEndings('a\nb', '\r\n')).toBe('a\r\nb')
  })

  it('leaves LF as-is when target is LF', () => {
    expect(restoreLineEndings('a\nb', '\n')).toBe('a\nb')
  })
})

describe('isBinaryPath', () => {
  it('detects image files', () => {
    expect(isBinaryPath('photo.png')).toBe(true)
    expect(isBinaryPath('image.jpg')).toBe(true)
  })

  it('detects archives', () => {
    expect(isBinaryPath('archive.zip')).toBe(true)
    expect(isBinaryPath('package.tar.gz')).toBe(true)
  })

  it('returns false for text files', () => {
    expect(isBinaryPath('code.ts')).toBe(false)
    expect(isBinaryPath('readme.md')).toBe(false)
    expect(isBinaryPath('data.json')).toBe(false)
  })

  it('returns false for extensionless files', () => {
    expect(isBinaryPath('Makefile')).toBe(false)
  })
})

describe('resolveWorkspacePath', () => {
  let tmpDir: string
  let outsideDir: string

  beforeEach(async () => {
    const suffix = `${Date.now()}-${Math.random().toString(36).slice(2)}`
    tmpDir = join(tmpdir(), `fs-path-test-${suffix}`)
    outsideDir = join(tmpdir(), `fs-path-outside-${suffix}`)
    await mkdir(tmpDir, { recursive: true })
    await mkdir(outsideDir, { recursive: true })
  })

  afterEach(async () => {
    await rm(tmpDir, { recursive: true, force: true })
    await rm(outsideDir, { recursive: true, force: true })
  })

  it('resolves relative paths inside the workspace', async () => {
    await writeFile(join(tmpDir, 'inside.txt'), 'ok')

    const resolved = await resolveWorkspacePath(tmpDir, 'inside.txt')

    expect(resolved).toBe(join(tmpDir, 'inside.txt'))
  })

  it('allows absolute paths inside the workspace', async () => {
    const filePath = join(tmpDir, 'absolute.txt')
    await writeFile(filePath, 'ok')

    const resolved = await resolveWorkspacePath(tmpDir, filePath)

    expect(resolved).toBe(filePath)
  })

  it('rejects parent traversal outside the workspace', async () => {
    await expect(
      resolveWorkspacePath(tmpDir, '../outside.txt'),
    ).rejects.toThrow(WORKSPACE_PATH_ERROR)
  })

  it('rejects absolute paths outside the workspace', async () => {
    const filePath = join(outsideDir, 'secret.txt')
    await writeFile(filePath, 'secret')

    await expect(resolveWorkspacePath(tmpDir, filePath)).rejects.toThrow(
      WORKSPACE_PATH_ERROR,
    )
  })

  it('rejects symlinked files outside the workspace', async () => {
    const filePath = join(outsideDir, 'secret.txt')
    await writeFile(filePath, 'secret')
    await symlink(filePath, join(tmpDir, 'secret-link.txt'))

    await expect(
      resolveWorkspacePath(tmpDir, 'secret-link.txt'),
    ).rejects.toThrow(WORKSPACE_PATH_ERROR)
  })

  it('rejects symlinked directories outside the workspace', async () => {
    await symlink(outsideDir, join(tmpDir, 'external-dir'), 'dir')

    await expect(
      resolveWorkspacePath(tmpDir, 'external-dir/new.txt'),
    ).rejects.toThrow(WORKSPACE_PATH_ERROR)
  })

  it('allows symlinked files that stay inside the workspace', async () => {
    const filePath = join(tmpDir, 'target.txt')
    await writeFile(filePath, 'ok')
    await symlink(filePath, join(tmpDir, 'target-link.txt'))

    const resolved = await resolveWorkspacePath(tmpDir, 'target-link.txt')

    expect(resolved).toBe(join(tmpDir, 'target-link.txt'))
  })

  it('resolves the workspace root through symlinks', async () => {
    const root = await resolveWorkspaceRoot(tmpDir)

    expect(root).toBe(await realpath(tmpDir))
  })
})

describe('walkFiles', () => {
  let tmpDir: string
  let outsideDir: string

  beforeEach(async () => {
    const suffix = `${Date.now()}-${Math.random().toString(36).slice(2)}`
    tmpDir = join(tmpdir(), `fs-walk-test-${suffix}`)
    outsideDir = join(tmpdir(), `fs-walk-outside-${suffix}`)
    await mkdir(tmpDir, { recursive: true })
    await mkdir(outsideDir, { recursive: true })
  })

  afterEach(async () => {
    await rm(tmpDir, { recursive: true, force: true })
    await rm(outsideDir, { recursive: true, force: true })
  })

  it('walks files recursively', async () => {
    await mkdir(join(tmpDir, 'a', 'b'), { recursive: true })
    await writeFile(join(tmpDir, 'root.txt'), '')
    await writeFile(join(tmpDir, 'a', 'mid.txt'), '')
    await writeFile(join(tmpDir, 'a', 'b', 'deep.txt'), '')

    const files: string[] = []
    for await (const f of walkFiles(tmpDir, tmpDir)) {
      files.push(f)
    }
    expect(files).toContain('root.txt')
    expect(files).toContain(join('a', 'mid.txt'))
    expect(files).toContain(join('a', 'b', 'deep.txt'))
  })

  it('skips node_modules', async () => {
    await mkdir(join(tmpDir, 'node_modules', 'pkg'), { recursive: true })
    await writeFile(join(tmpDir, 'node_modules', 'pkg', 'index.js'), '')
    await writeFile(join(tmpDir, 'real.ts'), '')

    const files: string[] = []
    for await (const f of walkFiles(tmpDir, tmpDir)) {
      files.push(f)
    }
    expect(files).toContain('real.ts')
    expect(files.some((f) => f.includes('node_modules'))).toBe(false)
  })

  it('skips .git', async () => {
    await mkdir(join(tmpDir, '.git', 'objects'), { recursive: true })
    await writeFile(join(tmpDir, '.git', 'HEAD'), '')
    await writeFile(join(tmpDir, 'code.ts'), '')

    const files: string[] = []
    for await (const f of walkFiles(tmpDir, tmpDir)) {
      files.push(f)
    }
    expect(files).toContain('code.ts')
    expect(files.some((f) => f.includes('.git'))).toBe(false)
  })

  it('handles empty directories', async () => {
    const files: string[] = []
    for await (const f of walkFiles(tmpDir, tmpDir)) {
      files.push(f)
    }
    expect(files.length).toBe(0)
  })

  it('handles nonexistent directory gracefully', async () => {
    const files: string[] = []
    for await (const f of walkFiles(join(tmpDir, 'nonexistent'), tmpDir)) {
      files.push(f)
    }
    expect(files.length).toBe(0)
  })

  it('skips symlinks that resolve outside the boundary', async () => {
    const outsideFile = join(outsideDir, 'secret.txt')
    await writeFile(outsideFile, 'secret')
    await symlink(outsideFile, join(tmpDir, 'secret-link.txt'))
    await writeFile(join(tmpDir, 'visible.txt'), '')

    const files: string[] = []
    for await (const f of walkFiles(tmpDir, tmpDir, tmpDir)) {
      files.push(f)
    }

    expect(files).toContain('visible.txt')
    expect(files).not.toContain('secret-link.txt')
  })
})
