/**
 * @license
 * Copyright 2025 BrowserOS
 *
 * Tool-registry completeness checks.
 *
 * This previously held a STDIO-transport e2e suite (`--headless` / `--isolated`
 * CLI args) that was skipped after the Phase 4 HTTP/SSE migration. The live
 * end-to-end coverage (health, status, /mcp listTools + callTool, /chat) now
 * lives in `server.integration.test.ts`, which drives the server over
 * StreamableHTTP against a real browser. What that suite does not assert — and
 * what the old "has all tools" case did — is that every tool is registered
 * exactly once. We keep that check here as a fast, browser-free unit test.
 */

import { describe, it } from 'bun:test'
import assert from 'node:assert'

import { registry } from '../src/tools/registry'

describe('tool registry', () => {
  it('exposes a non-empty tool set', () => {
    assert.ok(registry.all().length > 0, 'registry should expose tools')
  })

  it('every tool has a non-empty name', () => {
    for (const tool of registry.all()) {
      assert.ok(
        typeof tool.name === 'string' && tool.name.length > 0,
        `tool is missing a name: ${JSON.stringify(tool)}`,
      )
    }
  })

  it('tool names are unique and match the registered set', () => {
    const names = registry.names()
    const unique = new Set(names)
    assert.strictEqual(
      unique.size,
      names.length,
      `duplicate tool names: ${names.filter((n, i) => names.indexOf(n) !== i).join(', ')}`,
    )
    assert.strictEqual(
      names.length,
      registry.all().length,
      'names() and all() must describe the same tools',
    )
  })

  it('resolves a known tool by name', () => {
    assert.ok(registry.get('list_pages'), 'list_pages should be registered')
  })
})
