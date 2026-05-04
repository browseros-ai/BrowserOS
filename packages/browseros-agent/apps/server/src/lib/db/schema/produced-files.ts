/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { InferInsertModel, InferSelectModel } from 'drizzle-orm'
import {
  index,
  integer,
  sqliteTable,
  text,
  uniqueIndex,
} from 'drizzle-orm/sqlite-core'
import { agentDefinitions } from './agents'

/**
 * Files an OpenClaw agent produced as part of a chat turn. Populated by
 * a per-turn workspace diff: snapshot the agent's CWD before
 * `chatStream(...)` runs, re-scan after the SSE `done` event fires,
 * write rows for any new or modified path. The rail UI groups by
 * `turn_id` and the inline artifact card renders one row per file.
 *
 * Schema is intentionally adapter-agnostic — V1 only enables the
 * watcher for the openclaw adapter, but V2 can plug Claude / Codex
 * into the same table without migrating.
 */
export const producedFiles = sqliteTable(
  'produced_files',
  {
    /** Stable id; opaque file handle in download/preview URLs. */
    id: text('id').primaryKey(),

    /** FK → agent_definitions.id; CASCADE so agent deletion sweeps. */
    agentDefinitionId: text('agent_definition_id')
      .notNull()
      .references(() => agentDefinitions.id, { onDelete: 'cascade' }),

    /** OpenClaw session that owns this turn (e.g. `session-abc`). */
    sessionKey: text('session_key').notNull(),

    /** Identifier for the assistant turn that produced the file. */
    turnId: text('turn_id').notNull(),

    /**
     * The user prompt that initiated this turn — denormalised so the
     * rail's "group by prompt" header doesn't have to join the JSONL
     * log. Capped at 280 chars in code; the column is unbounded.
     */
    turnPrompt: text('turn_prompt').notNull(),

    /** Workspace-relative path (e.g. `reports/q1.pdf`). */
    path: text('path').notNull(),

    size: integer('size').notNull(),

    /** mtime in ms — used to detect re-modifications. */
    mtimeMs: integer('mtime_ms').notNull(),

    /** Server clock when our watcher first saw it. */
    createdAt: integer('created_at').notNull(),

    /**
     * `'diff'` for the V1 per-turn workspace diff watcher;
     * `'tool'` reserved for the future tool-event parsing layer.
     */
    detectedBy: text('detected_by', { enum: ['diff', 'tool'] })
      .notNull()
      .default('diff'),
  },
  (table) => [
    // One row per (agent, path) pair — re-modifications update in place,
    // so a tool that overwrites `report.pdf` doesn't accumulate
    // duplicate rows. The most recent turn that touched the file owns
    // the row.
    uniqueIndex('produced_files_agent_path_unique').on(
      table.agentDefinitionId,
      table.path,
    ),
    // Outputs-rail query: latest files per agent.
    index('produced_files_agent_created_idx').on(
      table.agentDefinitionId,
      table.createdAt,
    ),
    // Inline-card query: by turn.
    index('produced_files_turn_idx').on(table.turnId),
    // Cleanup hook: by session (when a session is deleted later).
    index('produced_files_session_idx').on(table.sessionKey),
  ],
)

export type ProducedFileRow = InferSelectModel<typeof producedFiles>
export type NewProducedFileRow = InferInsertModel<typeof producedFiles>
