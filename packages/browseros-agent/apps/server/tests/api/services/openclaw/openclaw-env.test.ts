/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import {
  getHostWorkspaceDir,
  isAgentWorkspaceNameSafe,
  mergeEnvContent,
} from '../../../../src/api/services/openclaw/openclaw-env'

describe('isAgentWorkspaceNameSafe', () => {
  it('accepts plain slugs', () => {
    expect(isAgentWorkspaceNameSafe('agent-01')).toBe(true)
    expect(isAgentWorkspaceNameSafe('research_bot')).toBe(true)
    expect(isAgentWorkspaceNameSafe('My Agent')).toBe(true)
  })

  it('rejects empty or whitespace-only', () => {
    expect(isAgentWorkspaceNameSafe('')).toBe(false)
    expect(isAgentWorkspaceNameSafe('   ')).toBe(false)
  })

  it('rejects path-traversal segments', () => {
    expect(isAgentWorkspaceNameSafe('..')).toBe(false)
    expect(isAgentWorkspaceNameSafe('../tmp')).toBe(false)
    expect(isAgentWorkspaceNameSafe('foo/../bar')).toBe(false)
    expect(isAgentWorkspaceNameSafe('foo..bar')).toBe(false)
  })

  it('rejects path separators and NULs', () => {
    expect(isAgentWorkspaceNameSafe('foo/bar')).toBe(false)
    expect(isAgentWorkspaceNameSafe('foo\\bar')).toBe(false)
    expect(isAgentWorkspaceNameSafe('foo\0bar')).toBe(false)
  })

  it('rejects names that start with a dot (hidden / dotfile)', () => {
    expect(isAgentWorkspaceNameSafe('.hidden')).toBe(false)
    expect(isAgentWorkspaceNameSafe('.')).toBe(false)
  })

  it('rejects control characters', () => {
    expect(isAgentWorkspaceNameSafe('foo\nbar')).toBe(false)
    expect(isAgentWorkspaceNameSafe('foo\x07bar')).toBe(false)
  })
})

describe('getHostWorkspaceDir', () => {
  it("returns the canonical 'main' workspace path", () => {
    expect(getHostWorkspaceDir('/tmp/openclaw', 'main')).toBe(
      '/tmp/openclaw/.openclaw/workspace',
    )
  })

  it('returns a per-agent workspace for safe names', () => {
    expect(getHostWorkspaceDir('/tmp/openclaw', 'agent-01')).toBe(
      '/tmp/openclaw/.openclaw/workspace-agent-01',
    )
  })

  it('throws for path-traversal names instead of escaping the state dir', () => {
    expect(() => getHostWorkspaceDir('/tmp/openclaw', '../../etc')).toThrow(
      /unsafe agent name/i,
    )
  })

  it('throws for names containing path separators', () => {
    expect(() => getHostWorkspaceDir('/tmp/openclaw', 'foo/bar')).toThrow(
      /unsafe agent name/i,
    )
  })
})

describe('mergeEnvContent', () => {
  it('appends new env keys and normalizes trailing newline', () => {
    expect(
      mergeEnvContent('OPENAI_API_KEY=sk-old', {
        ANTHROPIC_API_KEY: 'ant-key',
      }),
    ).toEqual({
      changed: true,
      content: 'OPENAI_API_KEY=sk-old\nANTHROPIC_API_KEY=ant-key\n',
    })
  })

  it('overwrites existing keys when values change', () => {
    expect(
      mergeEnvContent('OPENAI_API_KEY=sk-old\n', {
        OPENAI_API_KEY: 'sk-new',
      }),
    ).toEqual({
      changed: true,
      content: 'OPENAI_API_KEY=sk-new\n',
    })
  })

  it('reports unchanged when incoming values match existing content', () => {
    expect(
      mergeEnvContent('OPENAI_API_KEY=sk-test\n', {
        OPENAI_API_KEY: 'sk-test',
      }),
    ).toEqual({
      changed: false,
      content: 'OPENAI_API_KEY=sk-test\n',
    })
  })
})
