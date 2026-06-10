/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Plants and refreshes the BrowserOS-managed instruction file inside
 * an ACP agent's workspace. Called from `createAcpLanguageModel`
 * when a new conversation starts; subsequent turns short-circuit.
 */

import { dirname, join } from 'node:path'
import { type BuildSystemPromptOptions, buildSystemPrompt } from '../prompt'
import { instructionFilenameFor } from './filenames'
import { promptHash } from './hash'
import {
  findManagedBlock,
  renderManagedBlock,
  spliceManagedBlock,
} from './managedBlock'

export interface EnsureInstructionFileOptions {
  workspacePath: string
  providerType: string
  promptOptions: BuildSystemPromptOptions
  isNewConversation: boolean
  /**
   * Filesystem read injected so tests can stub without mocking
   * `node:fs/promises` at the module level (which leaks across files
   * in bun's shared test process). Production callers leave it unset.
   */
  readFile?: (path: string) => Promise<string | null>
  /**
   * Atomic write injected for the same reason. Default writes to a
   * `.browseros-tmp` sibling and renames into place.
   */
  writeFileAtomic?: (path: string, contents: string) => Promise<void>
}

export type EnsureInstructionFileResult =
  | { action: 'skipped-not-new-conversation' }
  | { action: 'skipped-up-to-date'; filename: string; path: string }
  | { action: 'created'; filename: string; path: string }
  | { action: 'updated'; filename: string; path: string }
  | { action: 'appended'; filename: string; path: string }
  | { action: 'failed'; filename: string; path: string; error: Error }

type FileLock = Promise<unknown>
const fileLocks = new Map<string, FileLock>()

function withFileLock<T>(path: string, fn: () => Promise<T>): Promise<T> {
  const prev = fileLocks.get(path) ?? Promise.resolve()
  const next = prev.then(fn, fn)
  const tail: FileLock = next.then(
    () => undefined,
    () => undefined,
  )
  fileLocks.set(path, tail)
  // Drop the entry once the chain settles so the Map cannot grow
  // without bound across long-running processes. The identity check
  // makes sure a newer queued call that overwrote our tail keeps its
  // entry; we only clear when nobody else is waiting on this path.
  tail.then(() => {
    if (fileLocks.get(path) === tail) fileLocks.delete(path)
  })
  return next
}

async function defaultReadFile(path: string): Promise<string | null> {
  const fs = await import('node:fs/promises')
  try {
    return await fs.readFile(path, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return null
    throw err
  }
}

async function defaultWriteFileAtomic(
  path: string,
  contents: string,
): Promise<void> {
  const fs = await import('node:fs/promises')
  const tmp = join(dirname(path), `.${path.split('/').pop()}.browseros-tmp`)
  await fs.writeFile(tmp, contents, 'utf8')
  await fs.rename(tmp, path)
}

export async function ensureWorkspaceInstructionFile(
  opts: EnsureInstructionFileOptions,
): Promise<EnsureInstructionFileResult> {
  if (!opts.isNewConversation) {
    return { action: 'skipped-not-new-conversation' }
  }

  const filename = instructionFilenameFor(opts.providerType)
  const path = join(opts.workspacePath, filename)
  const readFile = opts.readFile ?? defaultReadFile
  const writeFileAtomic = opts.writeFileAtomic ?? defaultWriteFileAtomic

  return withFileLock(path, async () => {
    try {
      const prompt = buildSystemPrompt(opts.promptOptions)
      const hash = promptHash(prompt)
      const nextBlock = renderManagedBlock(prompt, hash)

      const existing = await readFile(path)
      if (existing === null) {
        await writeFileAtomic(path, `${nextBlock}\n`)
        return { action: 'created', filename, path }
      }

      const block = findManagedBlock(existing)
      if (block === null) {
        const separator = existing.endsWith('\n\n')
          ? ''
          : existing.endsWith('\n')
            ? '\n'
            : '\n\n'
        await writeFileAtomic(path, `${existing}${separator}${nextBlock}\n`)
        return { action: 'appended', filename, path }
      }

      if (block.storedHash === hash) {
        return { action: 'skipped-up-to-date', filename, path }
      }

      const next = spliceManagedBlock(existing, block, nextBlock)
      await writeFileAtomic(path, next)
      return { action: 'updated', filename, path }
    } catch (err) {
      return {
        action: 'failed',
        filename,
        path,
        error: err instanceof Error ? err : new Error(String(err)),
      }
    }
  })
}
