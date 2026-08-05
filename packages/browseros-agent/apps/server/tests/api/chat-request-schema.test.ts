/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { ChatRequestSchema } from '../../src/api/types'

describe('ChatRequestSchema agent targets', () => {
  it('accepts a BrowserOS target with ordinary provider configuration', () => {
    const parsed = ChatRequestSchema.parse({
      target: { type: 'browseros', providerId: 'provider-1' },
      conversationId: crypto.randomUUID(),
      message: 'hello',
      provider: 'openai',
      providerId: 'provider-1',
      model: 'gpt-5',
    })

    expect(parsed.target).toEqual({
      type: 'browseros',
      providerId: 'provider-1',
    })
  })

  it.each(['claude', 'codex'] as const)(
    'accepts a %s target without LLM provider fields',
    (type) => {
      const parsed = ChatRequestSchema.parse({
        target: { type, agentId: crypto.randomUUID() },
        conversationId: crypto.randomUUID(),
        message: 'hello',
      })

      expect(parsed.target.type).toBe(type)
      expect('provider' in parsed).toBe(false)
    },
  )

  it('rejects ACP fields disguised as an LLM provider request', () => {
    const parsed = ChatRequestSchema.safeParse({
      target: { type: 'browseros', providerId: 'provider-1' },
      conversationId: crypto.randomUUID(),
      message: 'hello',
      provider: 'claude-code',
      providerId: 'provider-1',
      model: 'opus',
      acpAgentId: 'claude',
    })

    expect(parsed.success).toBe(false)
  })
})
