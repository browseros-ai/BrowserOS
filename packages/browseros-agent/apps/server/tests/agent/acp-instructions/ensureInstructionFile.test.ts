/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { beforeEach, describe, expect, it } from 'bun:test'
import { ensureWorkspaceInstructionFile } from '../../../src/agent/acp-instructions/ensureInstructionFile'
import { promptHash } from '../../../src/agent/acp-instructions/hash'
import {
  findManagedBlock,
  renderManagedBlock,
} from '../../../src/agent/acp-instructions/managedBlock'
import { buildSystemPrompt } from '../../../src/agent/prompt'

interface FakeFs {
  files: Map<string, string>
  reads: string[]
  writes: Array<{ path: string; contents: string }>
  readFile: (path: string) => Promise<string | null>
  writeFileAtomic: (path: string, contents: string) => Promise<void>
}

function makeFakeFs(): FakeFs {
  const files = new Map<string, string>()
  const reads: string[] = []
  const writes: Array<{ path: string; contents: string }> = []
  return {
    files,
    reads,
    writes,
    readFile: async (path) => {
      reads.push(path)
      return files.has(path) ? (files.get(path) as string) : null
    },
    writeFileAtomic: async (path, contents) => {
      writes.push({ path, contents })
      files.set(path, contents)
    },
  }
}

let fs: FakeFs

beforeEach(() => {
  fs = makeFakeFs()
})

const baseOpts = {
  workspacePath: '/tmp/ws',
  providerType: 'claude-code',
  promptOptions: { workspaceDir: '/tmp/ws' },
  isNewConversation: true,
}

describe('ensureWorkspaceInstructionFile', () => {
  it('writes a fresh CLAUDE.md when the workspace has no file yet', async () => {
    const result = await ensureWorkspaceInstructionFile({
      ...baseOpts,
      readFile: fs.readFile,
      writeFileAtomic: fs.writeFileAtomic,
    })

    expect(result.action).toBe('created')
    expect(fs.writes).toHaveLength(1)
    expect(fs.writes[0].path).toBe('/tmp/ws/CLAUDE.md')
    const block = findManagedBlock(fs.writes[0].contents)
    expect(block).not.toBeNull()
    const expectedHash = promptHash(buildSystemPrompt(baseOpts.promptOptions))
    expect(block?.storedHash).toBe(expectedHash)
  })

  it('uses AGENTS.md for codex', async () => {
    const result = await ensureWorkspaceInstructionFile({
      ...baseOpts,
      providerType: 'codex',
      readFile: fs.readFile,
      writeFileAtomic: fs.writeFileAtomic,
    })

    expect(result.action).toBe('created')
    expect(fs.writes[0].path).toBe('/tmp/ws/AGENTS.md')
  })

  it('uses AGENTS.md for acp-custom', async () => {
    const result = await ensureWorkspaceInstructionFile({
      ...baseOpts,
      providerType: 'acp-custom',
      readFile: fs.readFile,
      writeFileAtomic: fs.writeFileAtomic,
    })

    expect(result.action).toBe('created')
    expect(fs.writes[0].path).toBe('/tmp/ws/AGENTS.md')
  })

  it('appends to a user-authored file when no managed block exists', async () => {
    const userContent = '# My workspace notes\n\nthese are mine.\n'
    fs.files.set('/tmp/ws/CLAUDE.md', userContent)

    const result = await ensureWorkspaceInstructionFile({
      ...baseOpts,
      readFile: fs.readFile,
      writeFileAtomic: fs.writeFileAtomic,
    })

    expect(result.action).toBe('appended')
    expect(fs.writes).toHaveLength(1)
    const written = fs.writes[0].contents
    expect(written.startsWith(userContent)).toBe(true)
    expect(findManagedBlock(written)).not.toBeNull()
  })

  it('skips writing when the existing managed hash matches the rendered prompt', async () => {
    const prompt = buildSystemPrompt(baseOpts.promptOptions)
    const hash = promptHash(prompt)
    const existing = `${renderManagedBlock(prompt, hash)}\n`
    fs.files.set('/tmp/ws/CLAUDE.md', existing)

    const result = await ensureWorkspaceInstructionFile({
      ...baseOpts,
      readFile: fs.readFile,
      writeFileAtomic: fs.writeFileAtomic,
    })

    expect(result.action).toBe('skipped-up-to-date')
    expect(fs.writes).toHaveLength(0)
  })

  it('replaces the managed block when the stored hash differs and keeps surrounding bytes', async () => {
    const userTop = '# my workspace notes\n\n'
    const oldBlock = renderManagedBlock('old prompt body', 'deadbeef0000')
    const userBottom = '\n\n## addendum\nuser content.\n'
    fs.files.set('/tmp/ws/CLAUDE.md', userTop + oldBlock + userBottom)

    const result = await ensureWorkspaceInstructionFile({
      ...baseOpts,
      readFile: fs.readFile,
      writeFileAtomic: fs.writeFileAtomic,
    })

    expect(result.action).toBe('updated')
    const written = fs.writes[0].contents
    expect(written.startsWith(userTop)).toBe(true)
    expect(written.endsWith(userBottom)).toBe(true)
    expect(written).not.toContain('old prompt body')
    expect(written).not.toContain('deadbeef0000')
    const expectedHash = promptHash(buildSystemPrompt(baseOpts.promptOptions))
    expect(written).toContain(`<!-- BROWSEROS:HASH=${expectedHash} -->`)
  })

  it('does not even read the file when isNewConversation is false', async () => {
    const result = await ensureWorkspaceInstructionFile({
      ...baseOpts,
      isNewConversation: false,
      readFile: fs.readFile,
      writeFileAtomic: fs.writeFileAtomic,
    })

    expect(result.action).toBe('skipped-not-new-conversation')
    expect(fs.reads).toHaveLength(0)
    expect(fs.writes).toHaveLength(0)
  })

  it('serializes concurrent ensure calls against the same path', async () => {
    const writeOrder: number[] = []
    let inFlight = 0
    let maxInFlight = 0
    const slowWrite = async (path: string, contents: string) => {
      inFlight++
      maxInFlight = Math.max(maxInFlight, inFlight)
      await new Promise((resolve) => setTimeout(resolve, 5))
      writeOrder.push(contents.length)
      fs.files.set(path, contents)
      inFlight--
    }

    const promises = [
      ensureWorkspaceInstructionFile({
        ...baseOpts,
        readFile: fs.readFile,
        writeFileAtomic: slowWrite,
      }),
      ensureWorkspaceInstructionFile({
        ...baseOpts,
        readFile: fs.readFile,
        writeFileAtomic: slowWrite,
      }),
      ensureWorkspaceInstructionFile({
        ...baseOpts,
        readFile: fs.readFile,
        writeFileAtomic: slowWrite,
      }),
    ]
    const results = await Promise.all(promises)

    expect(maxInFlight).toBe(1)
    // First call creates the file; the next two see the managed block
    // they just wrote and short-circuit to skipped-up-to-date.
    expect(results[0].action).toBe('created')
    expect(results[1].action).toBe('skipped-up-to-date')
    expect(results[2].action).toBe('skipped-up-to-date')
    expect(writeOrder).toHaveLength(1)
  })

  it('returns a failed result instead of throwing when the write blows up', async () => {
    const result = await ensureWorkspaceInstructionFile({
      ...baseOpts,
      readFile: fs.readFile,
      writeFileAtomic: async () => {
        throw new Error('disk full')
      },
    })

    expect(result.action).toBe('failed')
    if (result.action === 'failed') {
      expect(result.error.message).toBe('disk full')
    }
  })
})
