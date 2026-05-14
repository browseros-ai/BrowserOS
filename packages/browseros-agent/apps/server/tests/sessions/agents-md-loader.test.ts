/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import fs from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'

import { AgentsMdLoader } from '../../src/sessions/agents-md-loader'

describe('AgentsMdLoader', () => {
  let tempDir: string
  let workspaceA: string
  let workspaceB: string
  let loader: AgentsMdLoader

  beforeEach(async () => {
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'agents-md-test-'))
    workspaceA = path.join(tempDir, 'workspace-a')
    workspaceB = path.join(tempDir, 'workspace-b')
    await fs.mkdir(workspaceA, { recursive: true })
    await fs.mkdir(workspaceB, { recursive: true })

    loader = new AgentsMdLoader([workspaceA, workspaceB])
  })

  afterEach(async () => {
    await fs.rm(tempDir, { recursive: true, force: true })
  })

  it('loads AGENTS.md from allowed workspace path', async () => {
    const agentsMdPath = path.join(workspaceA, 'AGENTS.md')
    await fs.writeFile(agentsMdPath, '# My Project\n\nSome rules here.', 'utf-8')

    const result = await loader.load(workspaceA)

    expect(result).not.toBeNull()
    expect(result!.path).toBe(agentsMdPath)
    expect(result!.content).toBe('# My Project\n\nSome rules here.')
    expect(result!.lastModified).toBeGreaterThan(0)
  })

  it('returns null for disallowed path', async () => {
    const otherDir = path.join(tempDir, 'other-workspace')
    await fs.mkdir(otherDir, { recursive: true })
    await fs.writeFile(path.join(otherDir, 'AGENTS.md'), '# Not allowed', 'utf-8')

    const result = await loader.load(otherDir)

    expect(result).toBeNull()
  })

  it('returns null when AGENTS.md does not exist', async () => {
    const result = await loader.load(workspaceA)

    expect(result).toBeNull()
  })

  it('rejects files exceeding 100KB size limit', async () => {
    const agentsMdPath = path.join(workspaceA, 'AGENTS.md')
    // 100KB = 102_400 bytes. Write 101KB.
    const oversized = 'x'.repeat(101 * 1024)
    await fs.writeFile(agentsMdPath, oversized, 'utf-8')

    const result = await loader.load(workspaceA)

    expect(result).toBeNull()
  })

  it('caches and detects changes via mtime', async () => {
    const agentsMdPath = path.join(workspaceA, 'AGENTS.md')
    await fs.writeFile(agentsMdPath, '# Version 1', 'utf-8')

    const result1 = await loader.load(workspaceA)
    expect(result1!.content).toBe('# Version 1')

    // Update the file
    await new Promise((r) => setTimeout(r, 50)) // ensure mtime changes
    await fs.writeFile(agentsMdPath, '# Version 2', 'utf-8')

    const result2 = await loader.load(workspaceA)
    expect(result2!.content).toBe('# Version 2')
  })

  it('loads multiple workspace AGENTS.md files', async () => {
    await fs.writeFile(path.join(workspaceA, 'AGENTS.md'), '# Workspace A', 'utf-8')
    await fs.writeFile(path.join(workspaceB, 'AGENTS.md'), '# Workspace B', 'utf-8')

    const results = await loader.loadMultiple([workspaceA, workspaceB])

    expect(results).toHaveLength(2)
    expect(results.map((r) => r.content).sort()).toEqual(['# Workspace A', '# Workspace B'])
  })

  it('prevents path traversal', async () => {
    const traversalPath = path.join(workspaceA, '..', '..', 'etc', 'passwd')
    const resolved = path.resolve(traversalPath)

    // Ensure /etc/passwd actually exists (or use a known existing file)
    let safeExistingDir: string
    try {
      await fs.access(resolved)
      safeExistingDir = resolved
    } catch {
      // /etc/passwd doesn't exist — create a target file in temp
      safeExistingDir = path.join(tempDir, 'secret')
      await fs.mkdir(safeExistingDir, { recursive: true })
      await fs.writeFile(path.join(safeExistingDir, 'passwd'), 'secret', 'utf-8')
    }

    const result = await loader.load(traversalPath)

    expect(result).toBeNull()
  })
})
