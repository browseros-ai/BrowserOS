/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * SQLite-backed store for files an OpenClaw agent produced inside its
 * workspace during a chat turn. The detection model is a per-turn
 * snapshot diff: take a `(path → size, mtime)` map of the workspace
 * before the turn starts, re-scan after the SSE `done` event, and
 * write a row for any new or modified file.
 *
 * Adapter-agnostic by design — the watcher is injected with the
 * agent's workspace dir, so V2 can plug Claude / Codex turn lifecycle
 * into the same store with a different `workspaceDir`.
 */

import { randomUUID } from 'node:crypto'
import { stat } from 'node:fs/promises'
import { relative, resolve, sep } from 'node:path'
import { desc, eq } from 'drizzle-orm'
import { type BrowserOsDatabase, getDb } from '../../../lib/db'
import {
  type NewProducedFileRow,
  type ProducedFileRow,
  producedFiles,
} from '../../../lib/db/schema'
import { walkWorkspace } from './produced-files-walker'

const TURN_PROMPT_MAX_CHARS = 280

export interface FileSnapshotEntry {
  size: number
  mtimeMs: number
}

/** A `(workspace-relative path → fs metadata)` snapshot of a workspace. */
export type FileSnapshot = Map<string, FileSnapshotEntry>

export interface FinalizeTurnInput {
  agentDefinitionId: string
  sessionKey: string
  turnId: string
  /** Raw user prompt; truncated to `TURN_PROMPT_MAX_CHARS` before persist. */
  turnPrompt: string
  /** Absolute host path to the agent's workspace directory. */
  workspaceDir: string
  /** Snapshot taken before the turn began. */
  before: FileSnapshot
}

export interface ResolvedFile {
  row: ProducedFileRow
  /** Absolute host path; guaranteed to live inside the original workspace. */
  absolutePath: string
}

export class ProducedFilesStore {
  private readonly db: BrowserOsDatabase

  constructor(options: { db?: BrowserOsDatabase } = {}) {
    this.db = options.db ?? getDb()
  }

  /**
   * Walk the workspace and capture every file's size + mtime. Used to
   * bracket a chat turn so the post-turn diff knows what changed.
   */
  async snapshotWorkspace(workspaceDir: string): Promise<FileSnapshot> {
    const snapshot: FileSnapshot = new Map()
    await walkWorkspace(workspaceDir, (relPath, metadata) => {
      snapshot.set(relPath, metadata)
    })
    return snapshot
  }

  /**
   * Diff the live workspace against `before`, persist rows for any
   * new or modified file, return the rows so the chat-turn finalizer
   * can broadcast them on the SSE feed. Re-modifications update the
   * existing row in place (the `(agentDefinitionId, path)` unique
   * index makes the upsert deterministic).
   */
  async finalizeTurn(input: FinalizeTurnInput): Promise<ProducedFileRow[]> {
    const after: FileSnapshot = await this.snapshotWorkspace(input.workspaceDir)
    const changed: Array<{ relPath: string; entry: FileSnapshotEntry }> = []
    for (const [relPath, entry] of after) {
      const previous = input.before.get(relPath)
      if (
        !previous ||
        previous.size !== entry.size ||
        previous.mtimeMs !== entry.mtimeMs
      ) {
        changed.push({ relPath, entry })
      }
    }
    if (changed.length === 0) return []

    const now = Date.now()
    const turnPrompt = truncatePrompt(input.turnPrompt)
    const rows: ProducedFileRow[] = []
    for (const { relPath, entry } of changed) {
      const row: NewProducedFileRow = {
        id: randomUUID(),
        agentDefinitionId: input.agentDefinitionId,
        sessionKey: input.sessionKey,
        turnId: input.turnId,
        turnPrompt,
        path: relPath,
        size: entry.size,
        mtimeMs: entry.mtimeMs,
        createdAt: now,
        detectedBy: 'diff',
      }
      // Upsert on (agent, path) — re-modifications win, no duplicates.
      const upserted = this.db
        .insert(producedFiles)
        .values(row)
        .onConflictDoUpdate({
          target: [producedFiles.agentDefinitionId, producedFiles.path],
          set: {
            sessionKey: row.sessionKey,
            turnId: row.turnId,
            turnPrompt: row.turnPrompt,
            size: row.size,
            mtimeMs: row.mtimeMs,
            createdAt: row.createdAt,
            detectedBy: row.detectedBy,
          },
        })
        .returning()
        .all()
      const persisted = upserted[0] ?? row
      rows.push(persisted as ProducedFileRow)
    }
    return rows
  }

  /** Inline-card query — files for a single assistant turn. */
  async listByTurn(turnId: string): Promise<ProducedFileRow[]> {
    return this.db
      .select()
      .from(producedFiles)
      .where(eq(producedFiles.turnId, turnId))
      .orderBy(desc(producedFiles.createdAt))
      .all()
  }

  /**
   * Outputs-rail query — every file an agent has produced across all
   * sessions, newest first.
   */
  async listByAgent(
    agentDefinitionId: string,
    options: { limit?: number } = {},
  ): Promise<ProducedFileRow[]> {
    const limit = options.limit ?? 200
    return this.db
      .select()
      .from(producedFiles)
      .where(eq(producedFiles.agentDefinitionId, agentDefinitionId))
      .orderBy(desc(producedFiles.createdAt))
      .limit(limit)
      .all()
  }

  /** Single-row lookup; null if the id is unknown. */
  async findById(id: string): Promise<ProducedFileRow | null> {
    const rows = this.db
      .select()
      .from(producedFiles)
      .where(eq(producedFiles.id, id))
      .limit(1)
      .all()
    return rows[0] ?? null
  }

  /** Used by `removeRegisteredModel` and similar admin paths later on. */
  async deleteByAgent(agentDefinitionId: string): Promise<void> {
    this.db
      .delete(producedFiles)
      .where(eq(producedFiles.agentDefinitionId, agentDefinitionId))
      .run()
  }

  /** Useful for hard-resetting a session's files (e.g. workspace clear). */
  async deleteBySession(sessionKey: string): Promise<void> {
    this.db
      .delete(producedFiles)
      .where(eq(producedFiles.sessionKey, sessionKey))
      .run()
  }

  /**
   * Resolve a stored file id to an absolute host path, after validating
   * that the on-disk path still lives inside `workspaceDir`. The HTTP
   * download / preview routes are the only callers; the workspace dir
   * is supplied by the openclaw service so this module stays
   * adapter-agnostic.
   */
  async resolveFilePath(input: {
    fileId: string
    workspaceDir: string
  }): Promise<ResolvedFile | null> {
    const row = await this.findById(input.fileId)
    if (!row) return null

    const workspaceRoot = resolve(input.workspaceDir)
    const absolutePath = resolve(workspaceRoot, row.path)

    // Containment check — reject anything escaping the workspace root.
    // `relative` returning a `..`-prefixed path means the resolved
    // absolute lives outside `workspaceRoot`.
    const rel = relative(workspaceRoot, absolutePath)
    if (rel.startsWith('..') || rel === '' || rel.startsWith(`..${sep}`)) {
      return null
    }

    // Final on-disk sanity check.
    try {
      await stat(absolutePath)
    } catch {
      return null
    }

    return { row, absolutePath }
  }

  /**
   * Group a flat list of rows by `turnId`, preserving the latest-first
   * order on the row level and keeping the most-recent group first.
   * The Outputs rail uses this shape directly.
   */
  groupByTurn(rows: ProducedFileRow[]): Array<{
    turnId: string
    turnPrompt: string
    createdAt: number
    files: ProducedFileRow[]
  }> {
    const grouped = new Map<
      string,
      {
        turnId: string
        turnPrompt: string
        createdAt: number
        files: ProducedFileRow[]
      }
    >()
    for (const row of rows) {
      const existing = grouped.get(row.turnId)
      if (!existing) {
        grouped.set(row.turnId, {
          turnId: row.turnId,
          turnPrompt: row.turnPrompt,
          // Group's createdAt = its newest file (rows are
          // already desc-by-createdAt, so the first one wins).
          createdAt: row.createdAt,
          files: [row],
        })
        continue
      }
      existing.files.push(row)
      if (row.createdAt > existing.createdAt) {
        existing.createdAt = row.createdAt
      }
    }
    return Array.from(grouped.values()).sort(
      (a, b) => b.createdAt - a.createdAt,
    )
  }
}

function truncatePrompt(value: string): string {
  const trimmed = value.trim()
  if (trimmed.length <= TURN_PROMPT_MAX_CHARS) return trimmed
  return `${trimmed.slice(0, TURN_PROMPT_MAX_CHARS - 1)}…`
}

// Re-export the row type so callers pulling the store don't have to
// also import the schema module.
export type { ProducedFileRow }
