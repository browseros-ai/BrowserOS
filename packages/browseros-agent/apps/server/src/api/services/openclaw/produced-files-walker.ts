/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Workspace walker used by the produced-files diff watcher. Recurses
 * an OpenClaw agent's workspace directory and yields one
 * `(workspace-relative path, size, mtime)` triple per file.
 *
 * Design choices:
 *
 * - **Pure async iteration.** No third-party deps; relies on
 *   `fs.promises.readdir` + `Dirent` so directory traversal is one
 *   syscall per directory.
 * - **Symlink-aware.** Symlinks themselves aren't followed (they
 *   appear in `Dirent.isSymbolicLink()`); the walker skips them so
 *   an agent can't smuggle host-fs paths into the diff via a
 *   symlink in its workspace.
 * - **Excludes well-known cruft directories** that no useful agent
 *   output ever lives inside (`node_modules`, `.git`, `.cache`).
 *   These directories are also expensive to traverse, so skipping
 *   them keeps the per-turn snapshot fast.
 * - **Bounded.** Hard caps on entry count and recursion depth keep
 *   pathological workspaces from stalling the chat-turn finalizer.
 */

import type { Dirent } from 'node:fs'
import { readdir, stat } from 'node:fs/promises'
import { join, relative, sep } from 'node:path'

const EXCLUDED_DIRECTORIES = new Set(['node_modules', '.git', '.cache'])

const MAX_ENTRIES = 50_000
const MAX_DEPTH = 16

export interface WorkspaceFileMetadata {
  size: number
  mtimeMs: number
}

export type WorkspaceFileVisitor = (
  /** Workspace-relative path (POSIX-style separators). */
  relativePath: string,
  metadata: WorkspaceFileMetadata,
) => void

/**
 * Walk `workspaceDir` recursively, calling `visit` for every regular
 * file. Returns silently if the directory doesn't exist (a fresh
 * agent that hasn't produced anything yet shouldn't error here).
 */
export async function walkWorkspace(
  workspaceDir: string,
  visit: WorkspaceFileVisitor,
): Promise<void> {
  let entriesSeen = 0
  await walk(workspaceDir, workspaceDir, 0, (file) => {
    entriesSeen += 1
    if (entriesSeen > MAX_ENTRIES) return false
    visit(file.relativePath, file.metadata)
    return true
  })
}

interface VisitedFile {
  relativePath: string
  metadata: WorkspaceFileMetadata
}

async function walk(
  root: string,
  current: string,
  depth: number,
  yieldFile: (file: VisitedFile) => boolean,
): Promise<boolean> {
  if (depth > MAX_DEPTH) return true

  let entries: Dirent[]
  try {
    entries = await readdir(current, { withFileTypes: true })
  } catch {
    // Workspace dir missing or unreadable — fresh agent that hasn't
    // written anything yet, or transient permissions issue. Treat as
    // "no files" rather than throwing.
    return true
  }

  for (const entry of entries) {
    if (EXCLUDED_DIRECTORIES.has(entry.name)) continue
    const absolute = join(current, entry.name)

    if (entry.isSymbolicLink()) {
      // Skip symlinks — never follow, never record. Prevents an
      // agent from smuggling host-fs paths into the diff via a
      // symlink in its workspace.
      continue
    }

    if (entry.isDirectory()) {
      const keepGoing = await walk(root, absolute, depth + 1, yieldFile)
      if (!keepGoing) return false
      continue
    }

    if (!entry.isFile()) continue

    let stats: Awaited<ReturnType<typeof stat>>
    try {
      stats = await stat(absolute)
    } catch {
      // Concurrent delete between readdir and stat — skip silently.
      continue
    }
    const relativePath = toPosix(relative(root, absolute))
    const keepGoing = yieldFile({
      relativePath,
      metadata: { size: stats.size, mtimeMs: stats.mtimeMs },
    })
    if (!keepGoing) return false
  }
  return true
}

function toPosix(value: string): string {
  if (sep === '/') return value
  return value.split(sep).join('/')
}
