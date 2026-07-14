import { describe, expect, test } from 'bun:test'
import type { ClientIdentity } from '../../../src/lib/mcp-session'
import { resolveAgentDisplay } from '../../../src/routes/tabs/agent-display'

function identity(
  p: Partial<ClientIdentity> & { sessionId: string },
): ClientIdentity {
  return {
    clientName: '',
    clientVersion: '',
    clientTitle: null,
    sessionLabel: null,
    firstSeenAt: 0,
    ...p,
  }
}

describe('resolveAgentDisplay', () => {
  test('identity prefers clientTitle and the colour matches the tab-group hex', () => {
    const result = resolveAgentDisplay(
      'claude-code',
      'claude-code',
      new Map([
        ['abc', identity({ sessionId: 's1', clientName: 'cursor' })],
        [
          'claude-code',
          identity({
            sessionId: 's1',
            clientName: 'claude-code',
            clientTitle: 'Claude Code',
          }),
        ],
      ]),
    )
    expect(result.agentLabel).toBe('Claude Code')
    expect(result.harness).toBeNull()
    expect(result.color).toMatch(/^#[0-9A-F]{6}$/)
  })

  test('identity falls back to clientName when title missing', () => {
    const result = resolveAgentDisplay(
      'claude-code',
      'claude-code',
      new Map([
        [
          'claude-code',
          identity({ sessionId: 's1', clientName: 'claude-code' }),
        ],
      ]),
    )
    expect(result.agentLabel).toBe('claude-code')
    expect(result.harness).toBeNull()
  })

  test('no identity falls back to slug and still emits a hex colour', () => {
    const result = resolveAgentDisplay(
      'unknown-abc123',
      'unknown-abc123',
      new Map(),
    )
    expect(result.agentLabel).toBe('unknown-abc123')
    expect(result.harness).toBeNull()
    expect(result.color).toMatch(/^#[0-9A-F]{6}$/)
  })
})
