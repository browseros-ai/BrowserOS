/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import {
  dispatchCommand,
  getAllAcpCommands,
  getAcpCommand,
  registerAcpCommand,
  clearAcpCommands,
  toAvailableCommands,
  type AcpCommandContext,
  type AcpCommandResult,
} from '../../../src/api/routes/acp-slash-commands'

// Re-register builtins after clear (for isolation)
import '../../../src/api/routes/acp-slash-commands-builtins'

describe('acp-slash-commands', () => {
  const baseCtx: Omit<AcpCommandContext, 'args'> = {
    agentId: 'agent-1',
    conversationId: 'conv-1',
    sessionId: 'main',
  }

  // ── dispatchCommand ────────────────────────────────────────────────

  describe('dispatchCommand', () => {
    it('returns passthrough for non-slash input', async () => {
      const result = await dispatchCommand('hello world', {
        ...baseCtx,
        args: '',
      })
      expect(result.type).toBe('passthrough')
    })

    it('returns passthrough for empty string', async () => {
      const result = await dispatchCommand('', { ...baseCtx, args: '' })
      expect(result.type).toBe('passthrough')
    })

    it('returns passthrough for plain text with slash mid-string', async () => {
      const result = await dispatchCommand('use /reset to clear', {
        ...baseCtx,
        args: '',
      })
      expect(result.type).toBe('passthrough')
    })

    it('dispatches known command', async () => {
      const result = await dispatchCommand('/help', { ...baseCtx, args: '' })
      expect(result.type).toBe('handled')
      if (result.type === 'handled') {
        expect(result.response).toContain('Available commands')
      }
    })

    it('passes args to command handler', async () => {
      const result = await dispatchCommand('/compact summarize this', {
        ...baseCtx,
        args: '',
      })
      expect(result.type).toBe('handled')
      if (result.type === 'handled') {
        expect(result.response).toContain('summarize this')
      }
    })

    it('returns error for unknown command', async () => {
      const result = await dispatchCommand('/foobar', {
        ...baseCtx,
        args: '',
      })
      expect(result.type).toBe('error')
      if (result.type === 'error') {
        expect(result.error).toContain('Unknown command')
        expect(result.error).toContain('/foobar')
        expect(result.error).toContain('/help')
      }
    })

    it('handles multiline args with [\\s\\S]* regex', async () => {
      const result = await dispatchCommand(
        '/compact first line\nsecond line',
        { ...baseCtx, args: '' },
      )
      expect(result.type).toBe('handled')
      if (result.type === 'handled') {
        expect(result.response).toContain('first line\nsecond line')
      }
    })

    it('handles empty args', async () => {
      const result = await dispatchCommand('/reset', { ...baseCtx, args: '' })
      expect(result.type).toBe('handled')
    })

    it('handles whitespace-only args as empty', async () => {
      const result = await dispatchCommand('/compact   ', {
        ...baseCtx,
        args: '',
      })
      expect(result.type).toBe('handled')
      if (result.type === 'handled') {
        expect(result.response).toContain('Compaction requested')
      }
    })
  })

  // ── Built-in commands ─────────────────────────────────────────────

  describe('/help', () => {
    it('lists all registered commands', async () => {
      const result = await dispatchCommand('/help', { ...baseCtx, args: '' })
      expect(result.type).toBe('handled')
      if (result.type === 'handled') {
        expect(result.response).toContain('/help')
        expect(result.response).toContain('/reset')
        expect(result.response).toContain('/compact')
        expect(result.response).toContain('/status')
      }
    })
  })

  describe('/reset', () => {
    it('returns handled with reset message', async () => {
      const result = await dispatchCommand('/reset', { ...baseCtx, args: '' })
      expect(result.type).toBe('handled')
      if (result.type === 'handled') {
        expect(result.response).toContain('reset')
      }
    })
  })

  describe('/compact', () => {
    it('without args returns general compaction message', async () => {
      const result = await dispatchCommand('/compact', {
        ...baseCtx,
        args: '',
      })
      expect(result.type).toBe('handled')
      if (result.type === 'handled') {
        expect(result.response).toContain('Compaction requested')
      }
    })

    it('with args returns focused compaction message', async () => {
      const result = await dispatchCommand('/compact API design', {
        ...baseCtx,
        args: '',
      })
      expect(result.type).toBe('handled')
      if (result.type === 'handled') {
        expect(result.response).toContain('API design')
      }
    })
  })

  describe('/status', () => {
    it('returns agent ID and session info', async () => {
      const result = await dispatchCommand('/status', { ...baseCtx, args: '' })
      expect(result.type).toBe('handled')
      if (result.type === 'handled') {
        expect(result.response).toContain('agent-1')
        expect(result.response).toContain('main')
        expect(result.response).toContain('conv-1')
      }
    })
  })

  // ── Registry functions ────────────────────────────────────────────

  describe('registry', () => {
    it('getAcpCommand returns undefined for unknown', () => {
      expect(getAcpCommand('nonexistent')).toBeUndefined()
    })

    it('getAllAcpCommands returns all built-in commands', () => {
      const all = getAllAcpCommands()
      const names = all.map((c) => c.name)
      expect(names).toContain('reset')
      expect(names).toContain('compact')
      expect(names).toContain('help')
      expect(names).toContain('status')
    })

    it('registerAcpCommand adds new command', () => {
      registerAcpCommand({
        name: 'test-registry-cmd',
        description: 'Test',
        usage: '/test-registry-cmd',
        async execute() {
          return { type: 'handled' as const, response: 'test' }
        },
      })
      expect(getAcpCommand('test-registry-cmd')).toBeDefined()
    })
  })

  // ── toAvailableCommands ───────────────────────────────────────────

  describe('toAvailableCommands', () => {
    it('returns commands in AvailableCommand format', () => {
      const cmds = toAvailableCommands()
      expect(Array.isArray(cmds)).toBe(true)
      expect(cmds.length).toBeGreaterThan(0)
      for (const cmd of cmds) {
        expect(cmd).toHaveProperty('name')
        expect(cmd).toHaveProperty('description')
        expect(typeof cmd.name).toBe('string')
        expect(typeof cmd.description).toBe('string')
      }
    })
  })
})
