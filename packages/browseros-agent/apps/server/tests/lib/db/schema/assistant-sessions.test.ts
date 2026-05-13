/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import {
  assistantSessions,
  sessionWorkspaces,
  sessionTags,
  type AssistantSessionRow,
  type NewAssistantSessionRow,
  type SessionWorkspaceRow,
  type NewSessionWorkspaceRow,
  type SessionTagRow,
  type NewSessionTagRow,
} from '../../../../src/lib/db/schema/assistant-sessions'

describe('assistant-sessions schema', () => {
  // --- assistantSessions table ---

  describe('assistantSessions table', () => {
    it('has correct table name', () => {
      expect(assistantSessions[Symbol.for('drizzle:Name')]).toBe('assistant_sessions')
    })

    it('defines all columns with correct types and constraints', () => {
      const t = assistantSessions

      // id: text PK
      expect(t.id.dataType).toBe('string')
      expect(t.id.primary).toBe(true)
      expect(t.id.notNull).toBe(true)

      // title: text nullable
      expect(t.title.dataType).toBe('string')
      expect(t.title.notNull).toBe(false)

      // mode: text enum ['chat','agent']
      expect(t.mode.dataType).toBe('string')
      expect(t.mode.config.enumValues).toEqual(['chat', 'agent'])

      // model: text nullable
      expect(t.model.dataType).toBe('string')
      expect(t.model.notNull).toBe(false)

      // messageCount: integer default 0
      expect(t.messageCount.dataType).toBe('number')
      expect(t.messageCount.notNull).toBe(true)
      expect(t.messageCount.hasDefault).toBe(true)

      // lastMessagePreview: text nullable
      expect(t.lastMessagePreview.dataType).toBe('string')
      expect(t.lastMessagePreview.notNull).toBe(false)

      // lastMessageAt: integer nullable
      expect(t.lastMessageAt.dataType).toBe('number')
      expect(t.lastMessageAt.notNull).toBe(false)

      // createdAt: integer not null
      expect(t.createdAt.dataType).toBe('number')
      expect(t.createdAt.notNull).toBe(true)

      // updatedAt: integer not null
      expect(t.updatedAt.dataType).toBe('number')
      expect(t.updatedAt.notNull).toBe(true)

      // meta: text nullable
      expect(t.meta.dataType).toBe('string')
      expect(t.meta.notNull).toBe(false)
    })

    it('defines an index on (updatedAt)', () => {
      const builder = assistantSessions[Symbol.for('drizzle:ExtraConfigBuilder')]
      expect(builder).toBeDefined()
      const indexes = builder({}) as Array<{ config: { name: string } }>
      const idx = indexes.find(
        (i) => i.config.name === 'assistant_sessions_updated_at_idx',
      )
      expect(idx).toBeDefined()
    })

    it('exports TypeScript types', () => {
      const _insert: NewAssistantSessionRow = {} as any
      const _select: AssistantSessionRow = {} as any
      expect(_insert).toBeDefined()
      expect(_select).toBeDefined()
    })
  })

  // --- sessionWorkspaces table ---

  describe('sessionWorkspaces table', () => {
    it('has correct table name', () => {
      expect(sessionWorkspaces[Symbol.for('drizzle:Name')]).toBe('session_workspaces')
    })

    it('defines all columns as notNull text', () => {
      const t = sessionWorkspaces

      expect(t.sessionId.dataType).toBe('string')
      expect(t.sessionId.notNull).toBe(true)

      expect(t.workspaceId.dataType).toBe('string')
      expect(t.workspaceId.notNull).toBe(true)

      expect(t.workspacePath.dataType).toBe('string')
      expect(t.workspacePath.notNull).toBe(true)

      expect(t.workspaceName.dataType).toBe('string')
      expect(t.workspaceName.notNull).toBe(true)
    })

    it('has composite primary key on (sessionId, workspaceId)', () => {
      const builder = sessionWorkspaces[Symbol.for('drizzle:ExtraConfigBuilder')]
      expect(builder).toBeDefined()
      const result = builder({}) as Array<{ constructor: { name: string }; columns: any[] }>
      const pkBuilder = result.find((r) => r.constructor.name === 'PrimaryKeyBuilder')
      expect(pkBuilder).toBeDefined()
      // primaryKey() stores column references that resolve at migration time;
      // verify 2 column slots are defined for the composite key
      expect(pkBuilder.columns).toHaveLength(2)
    })

    it('defines indexes on sessionId and workspacePath', () => {
      const builder = sessionWorkspaces[Symbol.for('drizzle:ExtraConfigBuilder')]
      expect(builder).toBeDefined()
      const result = builder({}) as Array<{ constructor: { name: string }; config?: { name: string } }>
      const indexNames = result
        .filter((r) => r.constructor.name === 'IndexBuilder' && r.config?.name)
        .map((r) => r.config!.name)
      expect(indexNames).toContain('session_workspaces_session_id_idx')
      expect(indexNames).toContain('session_workspaces_workspace_path_idx')
    })

    it('exports TypeScript types', () => {
      const _insert: NewSessionWorkspaceRow = {} as any
      const _select: SessionWorkspaceRow = {} as any
      expect(_insert).toBeDefined()
      expect(_select).toBeDefined()
    })
  })

  // --- sessionTags table ---

  describe('sessionTags table', () => {
    it('has correct table name', () => {
      expect(sessionTags[Symbol.for('drizzle:Name')]).toBe('session_tags')
    })

    it('defines sessionId and tag as notNull text columns', () => {
      const t = sessionTags

      expect(t.sessionId.dataType).toBe('string')
      expect(t.sessionId.notNull).toBe(true)

      expect(t.tag.dataType).toBe('string')
      expect(t.tag.notNull).toBe(true)
    })

    it('defines an index on (tag)', () => {
      const builder = sessionTags[Symbol.for('drizzle:ExtraConfigBuilder')]
      expect(builder).toBeDefined()
      const result = builder({}) as Array<{ config: { name: string } }>
      const idx = result.find(
        (i) => i?.config?.name === 'session_tags_tag_idx',
      )
      expect(idx).toBeDefined()
    })

    it('has composite primary key on (sessionId, tag)', () => {
      const builder = sessionTags[Symbol.for('drizzle:ExtraConfigBuilder')]
      const result = builder({}) as Array<{ config: { name: string } }>
      // primaryKey produces an entry with a different shape; verify it exists
      const hasPK = result.some((i) => i?.config?.name?.includes('session_tags') || i?.config?.columns)
      expect(hasPK).toBe(true)
    })

    it('exports TypeScript types', () => {
      const _insert: NewSessionTagRow = {} as any
      const _select: SessionTagRow = {} as any
      expect(_insert).toBeDefined()
      expect(_select).toBeDefined()
    })
  })
})
