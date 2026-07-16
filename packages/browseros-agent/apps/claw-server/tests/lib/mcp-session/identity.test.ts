import { describe, expect, it } from 'bun:test'
import {
  agentIdentityFromClient,
  agentKeyFromClient,
  createIdentityService,
  slugifyClientName,
} from '../../../src/lib/mcp-session/identity'

describe('IdentityService', () => {
  function setup(options: { now?: () => number; random?: () => number } = {}) {
    return createIdentityService({
      now: options.now ?? (() => 1_000_000),
      random: options.random ?? (() => 0),
    })
  }

  it('registers a born-named identity with one key and agent id', () => {
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
      slug: 'claude-code',
      label: 'agile-alpaca',
      generatedLabel: 'agile-alpaca',
      key: 'claude-code-agile-alpaca',
      firstSeenAt: 1_000_000,
    })
    expect(record.label).not.toBeNull()
    expect(agentKeyFromClient(record)).toBe(record.key)
    expect(agentIdentityFromClient(record)).toEqual({
      agentId: record.key,
      slug: 'claude-code',
    })
    expect(svc.getIdentity('s1')).toEqual(record)
  })

  it('uses agent for an unusable client slug', () => {
    const svc = setup()
    const record = svc.registerInitialize({
      sessionId: 's1',
      clientInfo: { name: '!!!' },
    })

    expect(record.slug).toBe('agent')
    expect(record.key).toBe('agent-agile-alpaca')
    expect(agentIdentityFromClient(record)).toEqual({
      agentId: 'agent-agile-alpaca',
      slug: 'agent',
    })
  })

  it('mints distinct keys for concurrent sessions from one client', () => {
    const draws = [0, 0, 0.03, 0.03]
    const svc = setup({ random: () => draws.shift() ?? 0.03 })
    const first = svc.registerInitialize({
      sessionId: 's1',
      clientInfo: { name: 'Claude Code' },
    })
    const second = svc.registerInitialize({
      sessionId: 's2',
      clientInfo: { name: 'Claude Code' },
    })

    expect(first.key).toBe('claude-code-agile-alpaca')
    expect(second.key).not.toBe(first.key)
    expect(second.slug).toBe(first.slug)
  })

  it('returns the original identity for duplicate initialization', () => {
    const draws = [0, 0, 0.5, 0.5]
    const svc = setup({ random: () => draws.shift() ?? 0.5 })
    const first = svc.registerInitialize({
      sessionId: 's1',
      clientInfo: { name: 'Claude Code', version: '1.0.0' },
    })
    const duplicate = svc.registerInitialize({
      sessionId: 's1',
      clientInfo: { name: 'Different Client', version: '2.0.0' },
    })

    expect(duplicate).toBe(first)
    expect(duplicate.key).toBe('claude-code-agile-alpaca')
    expect(duplicate.clientVersion).toBe('1.0.0')
    expect(svc.list()).toEqual([first])
  })

  it('keeps ended keys reserved until retention cleanup forgets them', () => {
    let now = 1_000
    const svc = setup({ now: () => now })
    const first = svc.registerInitialize({
      sessionId: 's1',
      clientInfo: { name: 'Claude Code' },
    })

    now = 2_000
    expect(svc.endSession('s1')).toEqual(first)
    expect(svc.getIdentity('s1')).toBeNull()
    expect(svc.list()).toEqual([])
    expect(svc.listRetained()).toEqual([{ key: first.key, endedAt: 2_000 }])

    const second = svc.registerInitialize({
      sessionId: 's2',
      clientInfo: { name: 'Claude Code' },
    })
    expect(second.key).not.toBe(first.key)

    svc.forgetRetained(first.key)
    expect(svc.listRetained()).toEqual([])
  })

  it('renames only the mutable label', () => {
    const svc = setup()
    const record = svc.registerInitialize({
      sessionId: 's1',
      clientInfo: { name: 'Claude Code' },
    })

    svc.setLabel('s1', 'invoice-processing')
    expect(svc.getIdentity('s1')).toMatchObject({
      key: record.key,
      generatedLabel: 'agile-alpaca',
      label: 'invoice-processing',
    })
  })

  it('returns null for unknown sessions and ignores unknown renames', () => {
    const svc = setup()
    expect(svc.getIdentity('missing')).toBeNull()
    svc.setLabel('missing', 'invoice-processing')
    expect(svc.size()).toBe(0)
  })

  it('starts fresh sessions with the full rename-nudge budget', () => {
    const svc = setup()
    const record = svc.registerInitialize({
      sessionId: 's1',
      clientInfo: { name: 'Claude Code' },
    })
    expect(record.renameNudgesLeft).toBe(5)
  })

  it('grants five rename nudges then refuses', () => {
    const svc = setup()
    svc.registerInitialize({
      sessionId: 's1',
      clientInfo: { name: 'Claude Code' },
    })

    for (let index = 0; index < 5; index += 1) {
      expect(svc.takeRenameNudge('s1')).toBe(true)
    }
    expect(svc.takeRenameNudge('s1')).toBe(false)
    expect(svc.getIdentity('s1')?.renameNudgesLeft).toBe(0)
  })

  it('refuses rename nudges once the label diverges from the generated one', () => {
    const svc = setup()
    svc.registerInitialize({
      sessionId: 's1',
      clientInfo: { name: 'Claude Code' },
    })

    svc.setLabel('s1', 'invoice-processing')
    expect(svc.takeRenameNudge('s1')).toBe(false)
  })

  it('refuses rename nudges for unknown or unidentified sessions', () => {
    const svc = setup()
    expect(svc.takeRenameNudge('missing')).toBe(false)
    expect(svc.takeRenameNudge('')).toBe(false)
  })

  it('clear removes live identities and retained reservations', () => {
    const svc = setup()
    svc.registerInitialize({ sessionId: 's1', clientInfo: { name: 'a' } })
    svc.endSession('s1')
    svc.registerInitialize({ sessionId: 's2', clientInfo: { name: 'b' } })

    svc.clear()
    expect(svc.size()).toBe(0)
    expect(svc.listRetained()).toEqual([])
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
    expect(slugifyClientName('x'.repeat(120))).toHaveLength(64)
  })

  it('returns an empty string for pure-unicode or pure-symbol input', () => {
    expect(slugifyClientName('!!!')).toBe('')
    expect(slugifyClientName('日本語')).toBe('')
    expect(slugifyClientName('')).toBe('')
  })
})
