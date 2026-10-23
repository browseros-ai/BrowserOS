import { describe, expect, it } from 'bun:test'
import { agentIdentity } from './harness.types'

describe('agentIdentity', () => {
  it('prefers the exact harness, then a known client prefix, then the label', () => {
    expect(agentIdentity('Codex', 'codex-mcp')).toEqual({
      harness: 'Codex',
      name: 'Codex',
    })
    expect(agentIdentity(undefined, 'claude')).toEqual({
      harness: 'Claude Code',
      name: 'Claude',
    })
    expect(agentIdentity(undefined, 'Cursor')).toEqual({
      harness: 'Cursor',
      name: 'Cursor',
    })
    expect(agentIdentity(undefined, 'cowork-finance-ops')).toEqual({
      name: 'cowork-finance-ops',
    })
  })
})
