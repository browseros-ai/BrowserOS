import { describe, expect, test } from 'bun:test'

import { CATALOG, CATALOG_BY_ID } from '../../src/_catalog/client-configs'
import { validateCatalog } from '../../src/_catalog/validate'

const NOW = new Date('2026-07-06T00:00:00Z')

describe('populated catalog', () => {
  test('every entry passes the validator against a recent NOW', () => {
    const errors = validateCatalog(CATALOG, NOW)
    expect(errors).toEqual([])
  })

  test('ships 23 clients matching Smithery roster + docker-known set', () => {
    expect(CATALOG).toHaveLength(23)
    expect(CATALOG.map((e) => e.id).sort()).toEqual([
      'amazon-bedrock',
      'amazonq',
      'antigravity',
      'boltai',
      'claude-code',
      'claude-desktop',
      'cline',
      'codex',
      'cursor',
      'enconvo',
      'gemini',
      'goose',
      'kiro',
      'librechat',
      'opencode',
      'roocode',
      'tome',
      'trae',
      'vscode',
      'vscode-insiders',
      'windsurf',
      'witsy',
      'zed',
    ])
  })

  test('CATALOG_BY_ID lookup returns the right entry per id', () => {
    for (const entry of CATALOG) {
      expect(CATALOG_BY_ID[entry.id]).toBe(entry)
    }
  })

  test('clients using serverUrl URL rename are captured (windsurf, antigravity)', () => {
    expect(CATALOG_BY_ID.windsurf.http?.urlField).toBe('serverUrl')
    expect(CATALOG_BY_ID.antigravity.http?.urlField).toBe('serverUrl')
  })

  test('clients using non-default HTTP tag values are captured', () => {
    expect(CATALOG_BY_ID.cline.http?.tagValue).toBe('streamableHttp')
    expect(CATALOG_BY_ID.kiro.http?.tagValue).toBe('streamable-http')
    expect(CATALOG_BY_ID.opencode.http?.tagValue).toBe('remote')
  })

  test('goose keeps its stdio quirks: cmd/envs renames, simpleName transform, type: stdio tag', () => {
    const g = CATALOG_BY_ID.goose.stdio
    expect(g.commandField).toBe('cmd')
    expect(g.envField).toBe('envs')
    expect(g.keyTransform).toBe('simpleName')
    expect(g.tagKey).toBe('type')
    expect(g.tagValue).toBe('stdio')
  })

  test('opencode: commandAsArray, env rename, type: local stdio inject', () => {
    const o = CATALOG_BY_ID.opencode.stdio
    expect(o.commandAsArray).toBe(true)
    expect(o.envField).toBe('environment')
    expect(o.injects).toEqual({ type: 'local', enabled: true })
  })

  test('claude-desktop is stdio-only (guards against the v0.0.2 regression)', () => {
    const cd = CATALOG_BY_ID['claude-desktop']
    expect(cd.supportedTransports.system).toEqual(['stdio'])
    expect(cd.http).toBeUndefined()
  })

  test('claude-code project scope forces stdio-only with a type tag', () => {
    const cc = CATALOG_BY_ID['claude-code']
    expect(cc.projectFile).toBe('.mcp.json')
    expect(cc.supportedTransports.project).toEqual(['stdio'])
    expect(cc.project?.stdio.tagKey).toBe('type')
    expect(cc.project?.stdio.tagValue).toBe('stdio')
  })

  test('claude-code system HTTP shape writes type: "http" and type: "sse" per transport', () => {
    // Regression: Claude Code emits a parse warning for HTTP entries
    // that lack an explicit `type` field. Without this shape, the
    // library wrote entries Claude Code silently rejected on launch.
    const cc = CATALOG_BY_ID['claude-code']
    expect(cc.http?.tagKey).toBe('type')
    expect(cc.http?.tagValue).toBe('http')
    expect(cc.http?.sseTagValue).toBe('sse')
  })
})
