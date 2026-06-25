import { describe, expect, it } from 'bun:test'
import { buildAgentApiUrl } from './agent-api-url'
import { mapHarnessAgentToEntry } from './agent-harness-types'

describe('mapHarnessAgentToEntry', () => {
  it('maps created harness agents into chat-compatible entries', () => {
    expect(
      mapHarnessAgentToEntry({
        id: 'agent-1',
        name: 'Review bot',
        adapter: 'codex',
        modelId: 'gpt-5.5',
        reasoningEffort: 'medium',
        permissionMode: 'approve-all',
        sessionKey: 'agent:agent-1:main',
        createdAt: 1000,
        updatedAt: 1000,
      }),
    ).toEqual({
      agentId: 'agent-1',
      name: 'Review bot',
      workspace: 'codex:main',
      model: 'gpt-5.5',
      source: 'agent-harness',
    })
  })
})

describe('buildAgentApiUrl', () => {
  it('does not add a trailing slash for the harness root route', () => {
    expect(buildAgentApiUrl('http://127.0.0.1:9105', '/')).toBe(
      'http://127.0.0.1:9105/agents',
    )
    expect(buildAgentApiUrl('http://127.0.0.1:9105', '/adapters')).toBe(
      'http://127.0.0.1:9105/agents/adapters',
    )
  })
})
