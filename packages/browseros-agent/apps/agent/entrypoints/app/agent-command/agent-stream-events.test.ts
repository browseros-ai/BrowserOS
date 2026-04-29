import { describe, expect, it } from 'bun:test'
import { mapAgentHarnessToolStatus } from './agent-stream-events'

describe('mapAgentHarnessToolStatus', () => {
  it('normalizes ACP tool statuses for the chat renderer', () => {
    expect(mapAgentHarnessToolStatus('running')).toBe('running')
    expect(mapAgentHarnessToolStatus('completed')).toBe('completed')
    expect(mapAgentHarnessToolStatus('failed')).toBe('error')
    expect(mapAgentHarnessToolStatus('incomplete')).toBe('running')
    expect(mapAgentHarnessToolStatus(undefined)).toBe('running')
  })
})
