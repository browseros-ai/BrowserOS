/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import {
  agentSessions,
  type AgentSessionRow,
  type NewAgentSessionRow,
} from '../../../../src/lib/db/schema/agent-sessions'

describe('agent-sessions schema', () => {
  it('exports the agentSessions table with correct name', () => {
    expect(agentSessions[Symbol.for('drizzle:Name')]).toBe('agent_sessions')
  })

  it('defines all columns with correct types and constraints', () => {
    const t = agentSessions

    // id: text PK
    expect(t.id.dataType).toBe('string')
    expect(t.id.primary).toBe(true)
    expect(t.id.notNull).toBe(true)

    // agentId: text not null
    expect(t.agentId.dataType).toBe('string')
    expect(t.agentId.notNull).toBe(true)
    expect(t.agentId.primary).toBeFalsy()

    // title: text nullable
    expect(t.title.dataType).toBe('string')
    expect(t.title.notNull).toBe(false)

    // mode: text enum ['code', 'ask', 'agent']
    expect(t.mode.dataType).toBe('string')
    expect(t.mode.config.enumValues).toEqual(['code', 'ask', 'agent'])

    // model: text nullable
    expect(t.model.dataType).toBe('string')
    expect(t.model.notNull).toBe(false)

    // turnCount: integer default 0
    expect(t.turnCount.dataType).toBe('number')
    expect(t.turnCount.notNull).toBe(true)
    expect(t.turnCount.hasDefault).toBe(true)

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

  it('defines an index on (agentId, updatedAt)', () => {
    const builder = agentSessions[Symbol.for('drizzle:ExtraConfigBuilder')]
    expect(builder).toBeDefined()
    const indexes = builder({}) as Array<{ config: { name: string } }>
    expect(indexes).toBeDefined()
    expect(indexes.length).toBeGreaterThanOrEqual(1)
    const idx = indexes.find(
      (i) => i.config.name === 'agent_sessions_agent_updated_idx',
    )
    expect(idx).toBeDefined()
  })

  it('exports TypeScript types', () => {
    const _insert: NewAgentSessionRow = {} as any
    const _select: AgentSessionRow = {} as any
    expect(_insert).toBeDefined()
    expect(_select).toBeDefined()
  })
})
