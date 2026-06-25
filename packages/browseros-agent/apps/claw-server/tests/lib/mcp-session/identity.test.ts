import { describe, expect, it } from 'bun:test'
import {
  agentIdentityFromClient,
  createIdentityService,
  fallbackSlugForSession,
  slugifyClientName,
} from '../../../src/lib/mcp-session/identity'

describe('IdentityService', () => {
  function setup(nowMs = 1_000_000) {
    return createIdentityService({ now: () => nowMs })
  }

  it('registers an initialize and returns the record', () => {
    const svc = setup()
    const record = svc.registerInitialize({
      sessionId: 's1',
      clientInfo: {
        name: 'Claude Code',
        version: '1.4.2',
        title: 'Claude Code',
      },
    })
    expect(record).toMatchObject({
      sessionId: 's1',
      clientName: 'Claude Code',
      clientVersion: '1.4.2',
      clientTitle: 'Claude Code',
      firstSeenAt: 1_000_000,
    })
    expect(svc.getIdentity('s1')).toEqual(record)
  })

  it('overwrites on a duplicate sessionId rather than appending', () => {
    const svc = setup()
    svc.registerInitialize({
      sessionId: 's1',
      clientInfo: { name: 'Claude Code', version: '1.4.2' },
    })
    svc.registerInitialize({
      sessionId: 's1',
      clientInfo: { name: 'Cursor', version: '0.99.0' },
    })
    expect(svc.size()).toBe(1)
    expect(svc.getIdentity('s1')?.clientName).toBe('Cursor')
  })

  it('returns null for an unknown session id', () => {
    const svc = setup()
    expect(svc.getIdentity('nope')).toBeNull()
  })

  it('drops a session and forgets the record', () => {
    const svc = setup()
    svc.registerInitialize({ sessionId: 's1', clientInfo: { name: 'a' } })
    svc.dropSession('s1')
    expect(svc.getIdentity('s1')).toBeNull()
    expect(svc.size()).toBe(0)
  })

  it('clear empties everything', () => {
    const svc = setup()
    svc.registerInitialize({ sessionId: 's1', clientInfo: { name: 'a' } })
    svc.registerInitialize({ sessionId: 's2', clientInfo: { name: 'b' } })
    svc.clear()
    expect(svc.size()).toBe(0)
  })

  it('trims and stores empty clientInfo fields cleanly', () => {
    const svc = setup()
    const record = svc.registerInitialize({
      sessionId: 's1',
      clientInfo: { name: '  ', version: undefined, title: '  ' },
    })
    expect(record.clientName).toBe('')
    expect(record.clientVersion).toBe('')
    expect(record.clientTitle).toBeNull()
  })
})

describe('slugifyClientName', () => {
  it('lowercases and collapses runs of non-alphanumerics', () => {
    expect(slugifyClientName('Claude Code')).toBe('claude-code')
    expect(slugifyClientName('VS  Code 1.2.3')).toBe('vs-code-1-2-3')
  })

  it('trims leading and trailing hyphens', () => {
    expect(slugifyClientName('  -- Cursor -- ')).toBe('cursor')
  })

  it('caps the output at 64 characters', () => {
    const raw = 'x'.repeat(120)
    expect(slugifyClientName(raw)).toHaveLength(64)
  })

  it('returns an empty string for pure-unicode or pure-symbol input', () => {
    expect(slugifyClientName('!!!')).toBe('')
    expect(slugifyClientName('日本語')).toBe('')
    expect(slugifyClientName('')).toBe('')
  })
})

describe('fallbackSlugForSession', () => {
  it('produces a stable handle for the same session', () => {
    const a = fallbackSlugForSession('abc-def-123')
    const b = fallbackSlugForSession('abc-def-123')
    expect(a).toBe(b)
    expect(a).toMatch(/^unknown-[0-9a-f]{6}$/)
  })

  it('produces a different handle for different sessions', () => {
    expect(fallbackSlugForSession('session-1')).not.toBe(
      fallbackSlugForSession('session-2'),
    )
  })
})

describe('agentIdentityFromClient', () => {
  it('uses the cleaned client name when one is present', () => {
    const identity = {
      sessionId: 's1',
      clientName: 'Claude Code',
      clientVersion: '1.0.0',
      clientTitle: null,
      firstSeenAt: 0,
    }
    expect(agentIdentityFromClient(identity)).toEqual({
      agentId: 'claude-code',
      slug: 'claude-code',
    })
  })

  it('falls back to the synthetic handle when clientName is empty', () => {
    const identity = {
      sessionId: 'session-xyz',
      clientName: '',
      clientVersion: '',
      clientTitle: null,
      firstSeenAt: 0,
    }
    const result = agentIdentityFromClient(identity)
    expect(result.agentId).toMatch(/^unknown-[0-9a-f]{6}$/)
    expect(result.agentId).toBe(result.slug)
  })

  it('falls back to the synthetic handle when clientName is pure unicode', () => {
    const identity = {
      sessionId: 's1',
      clientName: '日本語',
      clientVersion: '',
      clientTitle: null,
      firstSeenAt: 0,
    }
    expect(agentIdentityFromClient(identity).agentId).toMatch(
      /^unknown-[0-9a-f]{6}$/,
    )
  })
})
