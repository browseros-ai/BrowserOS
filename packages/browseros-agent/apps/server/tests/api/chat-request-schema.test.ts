/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { ChatRequestSchema } from '../../src/api/types'

describe('ChatRequestSchema agent targets', () => {
  it('normalizes a legacy BrowserOS request without a target', () => {
    const parsed = ChatRequestSchema.parse({
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

  for (const providerId of [undefined, '']) {
    it(`uses the provider type when a legacy provider ID is ${providerId === undefined ? 'missing' : 'empty'}`, () => {
      const parsed = ChatRequestSchema.parse({
        conversationId: crypto.randomUUID(),
        message: 'hello',
        provider: 'openai',
        providerId,
        model: 'gpt-5',
      })

      expect(parsed.target).toEqual({
        type: 'browseros',
        providerId: 'openai',
      })
    })
  }

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

  for (const type of ['claude', 'codex'] as const) {
    it(`accepts a ${type} target without LLM provider fields`, () => {
      const parsed = ChatRequestSchema.parse({
        target: { type, agentId: crypto.randomUUID() },
        conversationId: crypto.randomUUID(),
        message: 'hello',
      })

      expect(parsed.target.type).toBe(type)
      expect('provider' in parsed).toBe(false)
    })
  }

  it('rejects a malformed explicit target', () => {
    const parsed = ChatRequestSchema.safeParse({
      target: { type: 'browseros' },
      conversationId: crypto.randomUUID(),
      message: 'hello',
      provider: 'openai',
      providerId: 'provider-1',
      model: 'gpt-5',
    })

    expect(parsed.success).toBe(false)
  })

  it('rejects an untargeted legacy ACP provider request', () => {
    const parsed = ChatRequestSchema.safeParse({
      conversationId: crypto.randomUUID(),
      message: 'hello',
      provider: 'claude-code',
      providerId: 'provider-1',
      model: 'opus',
      acpAgentId: 'claude',
    })

    expect(parsed.success).toBe(false)
  })

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
