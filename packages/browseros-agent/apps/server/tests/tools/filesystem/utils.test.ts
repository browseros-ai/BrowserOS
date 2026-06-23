import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdir, realpath, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { basename, join, resolve } from 'node:path'
import { getToolOutputDir } from '../../../src/lib/browseros-dir'
import {
  detectLineEnding,
  isBinaryPath,
  normalizeToLF,
  resolveBrowserToolOutputPath,
  resolveWorkspacePath,
  resolveWorkspaceWritePath,
  restoreLineEndings,
  stripBom,
  truncateHead,
  truncateLine,
  truncateTail,
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

describe('walkFiles', () => {
  let tmpDir: string

  beforeEach(async () => {
    tmpDir = join(
      tmpdir(),
      `fs-walk-test-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    )
    await mkdir(tmpDir, { recursive: true })
  })

  afterEach(async () => {
    await rm(tmpDir, { recursive: true, force: true })
  })

  it('walks files recursively', async () => {
    await mkdir(join(tmpDir, 'a', 'b'), { recursive: true })
    await writeFile(join(tmpDir, 'root.txt'), '')
    await writeFile(join(tmpDir, 'a', 'mid.txt'), '')
    await writeFile(join(tmpDir, 'a', 'b', 'deep.txt'), '')

    const files: string[] = []
    for await (const f of walkFiles(tmpDir, tmpDir)) {
      files.push(f.path)
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
      files.push(f.path)
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
      files.push(f.path)
    }
    expect(files).toContain('code.ts')
    expect(files.some((f) => f.includes('.git'))).toBe(false)
  })

  it('handles empty directories', async () => {
    const files: string[] = []
    for await (const f of walkFiles(tmpDir, tmpDir)) {
      files.push(f.path)
    }
    expect(files.length).toBe(0)
  })

  it('handles nonexistent directory gracefully', async () => {
    const files: string[] = []
    for await (const f of walkFiles(join(tmpDir, 'nonexistent'), tmpDir)) {
      files.push(f.path)
    }
    expect(files.length).toBe(0)
  })

  it('returns canonical read paths for workspace symlinks', async () => {
    await mkdir(join(tmpDir, 'target'), { recursive: true })
    const realFile = join(tmpDir, 'target', 'real.txt')
    await writeFile(realFile, 'ok')
    await symlink(realFile, join(tmpDir, 'link.txt'))

    const files: Array<{ path: string; realPath: string }> = []
    for await (const f of walkFiles(tmpDir, tmpDir)) {
      files.push(f)
    }

    const link = files.find((f) => f.path === 'link.txt')
    expect(link?.realPath).toBe(await realpath(realFile))
  })
})

describe('filesystem path boundaries', () => {
  let tmpDir: string
  let outsideDir: string
  let browserosDir: string
  let previousBrowserosDir: string | undefined

  beforeEach(async () => {
    previousBrowserosDir = process.env.BROWSEROS_DIR
    browserosDir = join(
      tmpdir(),
      `fs-boundary-browseros-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    )
    process.env.BROWSEROS_DIR = browserosDir
    tmpDir = join(
      tmpdir(),
      `fs-boundary-test-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    )
    outsideDir = join(
      tmpdir(),
      `fs-boundary-outside-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    )
    await mkdir(tmpDir, { recursive: true })
    await mkdir(outsideDir, { recursive: true })
    await mkdir(browserosDir, { recursive: true })
  })

  afterEach(async () => {
    await rm(tmpDir, { recursive: true, force: true })
    await rm(outsideDir, { recursive: true, force: true })
    await rm(browserosDir, { recursive: true, force: true })
    if (previousBrowserosDir === undefined) {
      delete process.env.BROWSEROS_DIR
    } else {
      process.env.BROWSEROS_DIR = previousBrowserosDir
    }
  })

  it('resolves relative workspace paths inside the workspace', async () => {
    await mkdir(join(tmpDir, 'sub'), { recursive: true })
    await writeFile(join(tmpDir, 'sub', 'file.txt'), 'ok')

    const expectedPath = await realpath(join(tmpDir, 'sub', 'file.txt'))
    await expect(resolveWorkspacePath(tmpDir, 'sub/file.txt')).resolves.toBe(
      expectedPath,
    )
  })

  it('rejects absolute workspace paths', async () => {
    await writeFile(join(tmpDir, 'file.txt'), 'ok')

    await expect(
      resolveWorkspacePath(tmpDir, join(tmpDir, 'file.txt')),
    ).rejects.toThrow('relative to the selected workspace')
  })

  it('rejects traversal outside the workspace', async () => {
    await writeFile(join(outsideDir, 'secret.txt'), 'secret')

    await expect(
      resolveWorkspacePath(tmpDir, `../${basename(outsideDir)}/secret.txt`),
    ).rejects.toThrow('outside the selected workspace')
  })

  it('allows workspace entries whose names start with dot dot text', async () => {
    await mkdir(join(tmpDir, '..notes'), { recursive: true })
    await writeFile(join(tmpDir, '..notes', 'file.txt'), 'ok')

    const expectedPath = await realpath(join(tmpDir, '..notes', 'file.txt'))
    await expect(
      resolveWorkspacePath(tmpDir, '..notes/file.txt'),
    ).resolves.toBe(expectedPath)
  })

  it('rejects symlinks that resolve outside the workspace', async () => {
    await writeFile(join(outsideDir, 'secret.txt'), 'secret')
    await symlink(join(outsideDir, 'secret.txt'), join(tmpDir, 'secret-link'))

    await expect(resolveWorkspacePath(tmpDir, 'secret-link')).rejects.toThrow(
      'outside the selected workspace',
    )
  })

  it('allows write targets under an existing workspace parent', async () => {
    await mkdir(join(tmpDir, 'reports'), { recursive: true })

    const expectedParent = await realpath(join(tmpDir, 'reports'))
    await expect(
      resolveWorkspaceWritePath(tmpDir, 'reports/new/deep.txt'),
    ).resolves.toBe(resolve(expectedParent, 'new', 'deep.txt'))
  })

  it('rejects write targets below symlinked parents outside the workspace', async () => {
    await symlink(outsideDir, join(tmpDir, 'outside-link'))

    await expect(
      resolveWorkspaceWritePath(tmpDir, 'outside-link/new.txt'),
    ).rejects.toThrow('outside the selected workspace')
  })

  it('accepts BrowserOS tool output paths under the dedicated output directory', async () => {
    const outputDir = await getToolOutputDir()
    const outputPath = join(outputDir, 'snapshot.md')
    await writeFile(outputPath, 'snapshot')
    const expectedPath = await realpath(outputPath)

    await expect(resolveBrowserToolOutputPath(outputPath)).resolves.toBe(
      expectedPath,
    )
  })

  it('rejects relative BrowserOS tool output paths', async () => {
    await expect(resolveBrowserToolOutputPath('snapshot.md')).rejects.toThrow(
      'absolute BrowserOS tool output path',
    )
  })

  it('rejects sibling BrowserOS state paths outside the output directory', async () => {
    const outputDir = await getToolOutputDir()
    const siblingPath = join(outputDir, '..', 'config.json')
    await mkdir(join(outputDir, '..'), { recursive: true })
    await writeFile(siblingPath, '{}')

    await expect(resolveBrowserToolOutputPath(siblingPath)).rejects.toThrow(
      'outside BrowserOS tool output',
    )
  })

  it('rejects symlinked BrowserOS tool output roots', async () => {
    const rawOutputDir = join(browserosDir, 'tool-output')
    await symlink(outsideDir, rawOutputDir)
    const outputPath = join(rawOutputDir, 'snapshot.md')
    await writeFile(join(outsideDir, 'snapshot.md'), 'snapshot')

    await expect(resolveBrowserToolOutputPath(outputPath)).rejects.toThrow(
      'BrowserOS tool output directory must be a real directory',
    )
  })
})
