import { describe, expect, it } from 'bun:test'
import { buildAgentApiUrl } from './agent-api-url'

describe('buildAgentApiUrl', () => {
  it('does not add a trailing slash for the agent root route', () => {
    expect(buildAgentApiUrl('http://127.0.0.1:9105', '/')).toBe(
      'http://127.0.0.1:9105/agents',
    )
    expect(buildAgentApiUrl('http://127.0.0.1:9105', '/agent-1')).toBe(
      'http://127.0.0.1:9105/agents/agent-1',
    )
  })
})
