import { describe, expect, it, beforeEach } from 'bun:test'
import type { UIMessage } from 'ai'
import { processSlashCommand, clearCommands, getAllCommands, getCommand } from './registry'
import { registerBuiltinCommands } from './builtins'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeUIMessage(role: 'user' | 'assistant', text: string): UIMessage {
  return {
    id: crypto.randomUUID(),
    role,
    parts: [{ type: 'text', text }],
    createdAt: new Date(),
  }
}

let resetCalled = false
let modeSetTo: string | null = null
let messagesReplaced: UIMessage[] | null = null

function makeDeps(overrides?: { messages?: UIMessage[] }) {
  resetCalled = false
  modeSetTo = null
  messagesReplaced = null

  return {
    messages: overrides?.messages ?? [makeUIMessage('user', 'hello'), makeUIMessage('assistant', 'hi')],
    conversationId: 'test-conv-1',
    setMessages: (msgs: UIMessage[]) => { messagesReplaced = msgs },
    resetConversation: () => { resetCalled = true },
    mode: 'chat' as const,
    setMode: (mode: 'chat' | 'agent') => { modeSetTo = mode },
  }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('slash-commands', () => {
  beforeEach(() => {
    clearCommands()
    registerBuiltinCommands()
  })

  // -------------------------------------------------------------------------
  // Registry
  // -------------------------------------------------------------------------

  describe('registry', () => {
    it('registers all 6 built-in commands', () => {
      const commands = getAllCommands()
      expect(commands.length).toBe(6)
      const names = commands.map((c) => c.name).sort()
      expect(names).toEqual(['compact', 'fork', 'help', 'mode', 'new', 'undo'])
    })

    it('getCommand is case-insensitive', () => {
      expect(getCommand('Compact')?.name).toBe('compact')
      expect(getCommand('COMPACT')?.name).toBe('compact')
      expect(getCommand('HELP')?.name).toBe('help')
    })
  })

  // -------------------------------------------------------------------------
  // processSlashCommand
  // -------------------------------------------------------------------------

  describe('processSlashCommand', () => {
    it('returns passthrough for non-slash input', () => {
      const result = processSlashCommand('hello world', makeDeps())
      expect(result).toEqual({ type: 'passthrough', text: 'hello world' })
    })

    it('returns passthrough for unknown command', () => {
      const result = processSlashCommand('/foobar', makeDeps())
      expect(result).toEqual({ type: 'passthrough', text: '/foobar' })
    })

    it('returns passthrough for double slash (escape)', () => {
      const result = processSlashCommand('//not-a-command', makeDeps())
      expect(result).toEqual({ type: 'passthrough', text: '//not-a-command' })
    })

    it('returns passthrough for slash followed by space', () => {
      const result = processSlashCommand('/ not-a-command', makeDeps())
      expect(result).toEqual({ type: 'passthrough', text: '/ not-a-command' })
    })

    it('handles empty args correctly', () => {
      const result = processSlashCommand('/compact', makeDeps())
      expect(result.type).toBe('prompt')
      if (result.type === 'prompt') {
        expect(result.expandedText).toContain('Summarize our conversation')
      }
    })

    it('handles whitespace-only args as empty', () => {
      const result = processSlashCommand('/compact   ', makeDeps())
      expect(result.type).toBe('prompt')
      if (result.type === 'prompt') {
        expect(result.expandedText).not.toContain('focusing on:')
      }
    })

    it('captures single-line args', () => {
      const result = processSlashCommand('/compact performance issues', makeDeps())
      expect(result.type).toBe('prompt')
      if (result.type === 'prompt') {
        expect(result.expandedText).toContain('focusing on: performance issues')
      }
    })

    it('captures multi-line args (CRITICAL: uses [\\s\\S]*)', () => {
      const input = '/compact line one\nline two\nline three'
      const result = processSlashCommand(input, makeDeps())
      expect(result.type).toBe('prompt')
      if (result.type === 'prompt') {
        expect(result.expandedText).toContain('line one\nline two\nline three')
      }
    })

    it('captures args with special characters', () => {
      const result = processSlashCommand('/compact @user #tag $money %pct', makeDeps())
      expect(result.type).toBe('prompt')
      if (result.type === 'prompt') {
        expect(result.expandedText).toContain('@user #tag $money %pct')
      }
    })

    it('is case-insensitive for command name', () => {
      const result = processSlashCommand('/COMPACT stuff', makeDeps())
      expect(result.type).toBe('prompt')
      if (result.type === 'prompt') {
        expect(result.expandedText).toContain('stuff')
      }
    })
  })

  // -------------------------------------------------------------------------
  // /new command
  // -------------------------------------------------------------------------

  describe('/new', () => {
    it('resets conversation', () => {
      const result = processSlashCommand('/new', makeDeps())
      expect(result).toEqual({ type: 'action', handled: true })
      expect(resetCalled).toBe(true)
    })
  })

  // -------------------------------------------------------------------------
  // /compact command
  // -------------------------------------------------------------------------

  describe('/compact', () => {
    it('returns summary prompt without topic', () => {
      const result = processSlashCommand('/compact', makeDeps())
      expect(result.type).toBe('prompt')
      if (result.type === 'prompt') {
        expect(result.expandedText).toContain('Summarize our conversation')
        expect(result.expandedText).not.toContain('focusing on:')
      }
    })

    it('returns summary prompt with topic', () => {
      const result = processSlashCommand('/compact bugs', makeDeps())
      expect(result.type).toBe('prompt')
      if (result.type === 'prompt') {
        expect(result.expandedText).toContain('focusing on: bugs')
      }
    })
  })

  // -------------------------------------------------------------------------
  // /help command
  // -------------------------------------------------------------------------

  describe('/help', () => {
    it('returns help text with all commands', () => {
      const result = processSlashCommand('/help', makeDeps())
      expect(result.type).toBe('prompt')
      if (result.type === 'prompt') {
        expect(result.expandedText).toContain('Available Commands')
        expect(result.expandedText).toContain('/new')
        expect(result.expandedText).toContain('/compact')
        expect(result.expandedText).toContain('/help')
        expect(result.expandedText).toContain('/mode')
      }
    })
  })

  // -------------------------------------------------------------------------
  // /mode command
  // -------------------------------------------------------------------------

  describe('/mode', () => {
    it('switches to agent mode', () => {
      const result = processSlashCommand('/mode agent', makeDeps())
      expect(result).toEqual({ type: 'action', handled: true })
      expect(modeSetTo).toBe('agent')
    })

    it('switches to chat mode', () => {
      const result = processSlashCommand('/mode chat', makeDeps())
      expect(result).toEqual({ type: 'action', handled: true })
      expect(modeSetTo).toBe('chat')
    })

    it('shows current mode when no valid arg', () => {
      const result = processSlashCommand('/mode', makeDeps())
      expect(result.type).toBe('prompt')
      if (result.type === 'prompt') {
        expect(result.expandedText).toContain('chat')
        expect(result.expandedText).toContain('/mode chat')
      }
    })

    it('shows current mode for invalid arg', () => {
      const result = processSlashCommand('/mode invalid', makeDeps())
      expect(result.type).toBe('prompt')
      if (result.type === 'prompt') {
        expect(result.expandedText).toContain('chat')
      }
    })
  })

  // -------------------------------------------------------------------------
  // /fork command
  // -------------------------------------------------------------------------

  describe('/fork', () => {
    it('returns guidance prompt', () => {
      const result = processSlashCommand('/fork', makeDeps())
      expect(result.type).toBe('prompt')
      if (result.type === 'prompt') {
        expect(result.expandedText).toContain('fork')
      }
    })
  })

  // -------------------------------------------------------------------------
  // /undo command
  // -------------------------------------------------------------------------

  describe('/undo', () => {
    it('returns guidance prompt', () => {
      const result = processSlashCommand('/undo', makeDeps())
      expect(result.type).toBe('prompt')
      if (result.type === 'prompt') {
        expect(result.expandedText).toContain('undo')
      }
    })
  })

  // -------------------------------------------------------------------------
  // Edge cases
  // -------------------------------------------------------------------------

  describe('edge cases', () => {
    it('handles empty string input', () => {
      const result = processSlashCommand('', makeDeps())
      expect(result).toEqual({ type: 'passthrough', text: '' })
    })

    it('handles slash only', () => {
      const result = processSlashCommand('/', makeDeps())
      expect(result).toEqual({ type: 'passthrough', text: '/' })
    })

    it('handles command with hyphen in name', () => {
      // Commands with hyphens should match if registered
      // Since we don't have one built-in, it should passthrough
      const result = processSlashCommand('/my-command arg', makeDeps())
      expect(result).toEqual({ type: 'passthrough', text: '/my-command arg' })
    })

    it('handles command with underscore in name', () => {
      const result = processSlashCommand('/my_command arg', makeDeps())
      expect(result).toEqual({ type: 'passthrough', text: '/my_command arg' })
    })

    it('allowDuringBusy is set on /new', () => {
      const cmd = getCommand('new')
      expect(cmd?.allowDuringBusy).toBe(true)
    })

    it('allowDuringBusy is undefined on /compact', () => {
      const cmd = getCommand('compact')
      expect(cmd?.allowDuringBusy).toBeUndefined()
    })
  })
})
