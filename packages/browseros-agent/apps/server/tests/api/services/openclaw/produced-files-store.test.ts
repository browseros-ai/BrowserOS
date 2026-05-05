/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtempSync } from 'node:fs'
import { mkdir, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { resolveSafeWorkspacePath } from '../../../../src/api/services/openclaw/produced-files-store'

describe('resolveSafeWorkspacePath', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  it('resolves a regular file inside the workspace', async () => {
    const root = mkTempDir()
    const target = join(root, 'output.txt')
    await writeFile(target, 'hello')

    const resolved = await resolveSafeWorkspacePath(root, 'output.txt')

    expect(resolved).not.toBeNull()
    expect(resolved).toContain('output.txt')
  })

  it('resolves a nested file using its workspace-relative path', async () => {
    const root = mkTempDir()
    const subdir = join(root, 'reports')
    await mkdir(subdir, { recursive: true })
    await writeFile(join(subdir, 'q1.csv'), 'a,b\n1,2')

    const resolved = await resolveSafeWorkspacePath(root, 'reports/q1.csv')

    expect(resolved).not.toBeNull()
    expect(resolved).toMatch(/reports\/q1\.csv$/)
  })

  it('rejects lexical traversal with `..` segments', async () => {
    const root = mkTempDir()
    // Sibling file lives next to the workspace root so the lexical
    // join lands on a real, readable file — proving the rejection
    // is from the containment check, not a missing-file fallback.
    const siblingDir = mkTempDir()
    await writeFile(join(siblingDir, 'secret.txt'), 'do not leak')

    const escapingRel = join('..', '..', 'secret.txt')

    const resolved = await resolveSafeWorkspacePath(root, escapingRel)

    expect(resolved).toBeNull()
  })

  it('rejects a symlink whose target lives outside the workspace', async () => {
    const root = mkTempDir()
    const outside = mkTempDir()
    const secret = join(outside, 'passwd')
    await writeFile(secret, 'shadow:contents')

    // Symlink inside the workspace pointing to the outside file.
    // The lexical path stays inside the root, but the realpath
    // resolution should still reject it.
    await symlink(secret, join(root, 'looks-local'))

    const resolved = await resolveSafeWorkspacePath(root, 'looks-local')

    expect(resolved).toBeNull()
  })

  it('returns null for a path that does not exist on disk', async () => {
    const root = mkTempDir()

    const resolved = await resolveSafeWorkspacePath(root, 'never-created.bin')

    expect(resolved).toBeNull()
  })

  it('returns null when the workspace root itself is the resolved path', async () => {
    const root = mkTempDir()

    // Empty rel-path collapses to the root — must not be downloadable.
    const resolved = await resolveSafeWorkspacePath(root, '')

    expect(resolved).toBeNull()
  })

  function mkTempDir(): string {
    const dir = mkdtempSync(join(tmpdir(), 'browseros-files-test-'))
    tempDirs.push(dir)
    return dir
  }
})
