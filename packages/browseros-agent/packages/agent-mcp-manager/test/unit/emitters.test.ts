import { describe, expect, test } from 'bun:test'

import {
  claudeDesktop,
  cline,
  codex,
  goose,
  opencode,
  vscode,
  windsurf,
  zed,
} from '../../src/_catalog/client-configs'
import { getEmitter } from '../../src/emitters/index'
import { InvalidServerSpecError } from '../../src/errors'
import type { McpServerSpec } from '../../src/types'

const STDIO_SPEC: McpServerSpec = {
  transport: 'stdio',
  command: 'gh-mcp',
  args: ['serve'],
  env: { KEY: 'val' },
}

const HTTP_SPEC: McpServerSpec = {
  transport: 'http',
  url: 'https://example.com/mcp',
  headers: { Authorization: 'Bearer x' },
}

describe('getEmitter: JSON family', () => {
  test('claude-desktop writes stdio entry under mcpServers', () => {
    const out = getEmitter(claudeDesktop).add('', 'gh', STDIO_SPEC)
    const parsed = JSON.parse(out)
    expect(parsed.mcpServers.gh).toEqual({
      command: 'gh-mcp',
      args: ['serve'],
      env: { KEY: 'val' },
    })
  })

  test('vscode injects type: stdio tag', () => {
    const out = getEmitter(vscode).add('', 'gh', STDIO_SPEC)
    const parsed = JSON.parse(out)
    expect(parsed.servers.gh).toEqual({
      command: 'gh-mcp',
      args: ['serve'],
      env: { KEY: 'val' },
      type: 'stdio',
    })
  })

  test('vscode HTTP entry gets type: http and default url field', () => {
    const out = getEmitter(vscode).add('', 'gh', HTTP_SPEC)
    const parsed = JSON.parse(out)
    expect(parsed.servers.gh).toEqual({
      url: 'https://example.com/mcp',
      headers: { Authorization: 'Bearer x' },
      type: 'http',
    })
  })

  test('zed writes context_servers with injects (source, enabled) around stdio', () => {
    const out = getEmitter(zed).add('', 'gh', STDIO_SPEC)
    const parsed = JSON.parse(out)
    expect(parsed.context_servers.gh).toEqual({
      command: 'gh-mcp',
      args: ['serve'],
      env: { KEY: 'val' },
      source: 'custom',
      enabled: true,
    })
  })

  test('cline HTTP uses type: streamableHttp (camelCase)', () => {
    const out = getEmitter(cline).add('', 'gh', HTTP_SPEC)
    const parsed = JSON.parse(out)
    expect(parsed.mcpServers.gh.type).toBe('streamableHttp')
    expect(parsed.mcpServers.gh.url).toBe('https://example.com/mcp')
  })

  test('windsurf HTTP renames url -> serverUrl', () => {
    const out = getEmitter(windsurf).add('', 'gh', HTTP_SPEC)
    const parsed = JSON.parse(out)
    expect(parsed.mcpServers.gh).toEqual({
      serverUrl: 'https://example.com/mcp',
      headers: { Authorization: 'Bearer x' },
    })
    expect(parsed.mcpServers.gh.url).toBeUndefined()
  })

  test('opencode commandAsArray + type=local + enabled=true injects + env->environment rename', () => {
    const out = getEmitter(opencode).add('', 'gh', STDIO_SPEC)
    const parsed = JSON.parse(out)
    expect(parsed.mcp.gh).toEqual({
      command: ['gh-mcp', 'serve'],
      environment: { KEY: 'val' },
      type: 'local',
      enabled: true,
    })
  })

  test('read returns the entry names under the correct parent key', () => {
    const out = getEmitter(vscode).add('', 'gh', STDIO_SPEC)
    expect(getEmitter(vscode).read(out)).toEqual(['gh'])
  })

  test('remove drops the entry and keeps siblings intact', () => {
    let raw = getEmitter(vscode).add('', 'gh', STDIO_SPEC)
    raw = getEmitter(vscode).add(raw, 'other', STDIO_SPEC)
    const after = getEmitter(vscode).remove(raw, 'gh')
    const parsed = JSON.parse(after)
    expect(parsed.servers.gh).toBeUndefined()
    expect(parsed.servers.other).toBeDefined()
  })

  test('claude-desktop throws when writing an http entry (no http shape)', () => {
    expect(() => getEmitter(claudeDesktop).add('', 'gh', HTTP_SPEC)).toThrow(
      InvalidServerSpecError,
    )
  })
})

describe('getEmitter: YAML family', () => {
  test('goose writes into extensions with cmd rename + simpleName transform + type=stdio tag', () => {
    const out = getEmitter(goose).add('', 'MCP_DOCKER', STDIO_SPEC)
    // Key gets simpleName-transformed to `mcpdocker`.
    expect(out).toContain('mcpdocker:')
    expect(out).toContain('cmd: gh-mcp')
    expect(out).toContain('type: stdio')
    // args + envs are present too.
    expect(out).toContain('- serve')
  })

  test('goose read returns transformed key', () => {
    const raw = getEmitter(goose).add('', 'MCP_DOCKER', STDIO_SPEC)
    expect(getEmitter(goose).read(raw)).toEqual(['mcpdocker'])
  })

  test('goose remove drops the transformed key', () => {
    let raw = getEmitter(goose).add('', 'MCP_DOCKER', STDIO_SPEC)
    raw = getEmitter(goose).add(raw, 'OTHER', STDIO_SPEC)
    const after = getEmitter(goose).remove(raw, 'MCP_DOCKER')
    expect(getEmitter(goose).read(after)).toEqual(['other'])
  })
})

describe('getEmitter: TOML family (codex)', () => {
  test('codex writes an mcp_servers.NAME table for stdio', () => {
    const out = getEmitter(codex).add('', 'gh', STDIO_SPEC)
    expect(out).toContain('[mcp_servers.gh]')
    expect(out).toContain('command = "gh-mcp"')
    expect(out).toMatch(/args = \[\s*"serve"\s*\]/)
  })

  test('codex HTTP entry writes url', () => {
    const out = getEmitter(codex).add('', 'gh', HTTP_SPEC)
    expect(out).toContain('url = "https://example.com/mcp"')
  })

  test('codex read enumerates mcp_servers keys', () => {
    let raw = getEmitter(codex).add('', 'a', STDIO_SPEC)
    raw = getEmitter(codex).add(raw, 'b', STDIO_SPEC)
    expect(getEmitter(codex).read(raw).sort()).toEqual(['a', 'b'])
  })

  test('codex remove drops the entry and clears the table if empty', () => {
    const raw = getEmitter(codex).add('', 'gh', STDIO_SPEC)
    const after = getEmitter(codex).remove(raw, 'gh')
    expect(after).not.toContain('[mcp_servers')
  })
})

describe('getEmitter: project-scope override', () => {
  test('vscode project scope uses the same shape as system', () => {
    const out = getEmitter(vscode, 'project').add('', 'gh', STDIO_SPEC)
    const parsed = JSON.parse(out)
    expect(parsed.servers.gh.type).toBe('stdio')
  })
})
