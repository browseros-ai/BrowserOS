import { describe, expect, test } from 'bun:test'
import { AgentTargetSchema } from './agent'

describe('AgentTargetSchema', () => {
  test('accepts the id owned by each target kind', () => {
    expect(
      AgentTargetSchema.parse({ type: 'browseros', providerId: 'provider-1' }),
    ).toEqual({ type: 'browseros', providerId: 'provider-1' })
    expect(
      AgentTargetSchema.parse({
        type: 'claude',
        agentId: '00000000-0000-4000-8000-000000000001',
      }),
    ).toEqual({
      type: 'claude',
      agentId: '00000000-0000-4000-8000-000000000001',
    })
    expect(
      AgentTargetSchema.parse({
        type: 'codex',
        agentId: '00000000-0000-4000-8000-000000000002',
      }),
    ).toEqual({
      type: 'codex',
      agentId: '00000000-0000-4000-8000-000000000002',
    })
  })

  test('rejects ids owned by another target kind', () => {
    expect(
      AgentTargetSchema.safeParse({ type: 'browseros', agentId: 'agent-1' })
        .success,
    ).toBe(false)
    expect(
      AgentTargetSchema.safeParse({ type: 'claude', providerId: 'provider-1' })
        .success,
    ).toBe(false)
  })
})
