/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import {
  resetMcpManagerForTesting,
  setMcpManagerForTesting,
} from '../../src/lib/mcp-manager'
import {
  installForAgent,
  uninstallForAgent,
} from '../../src/services/harness-install'
import { createStubMcpManager } from '../_helpers/stub-mcp-manager'

const URL = 'http://127.0.0.1:9200/mcp'

describe('harness install service', () => {
  beforeEach(() => resetMcpManagerForTesting())
  afterEach(() => resetMcpManagerForTesting())

  test('installForAgent on Codex writes a direct http spec to the codex agent', async () => {
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    const outcome = await installForAgent({
      slug: 'cdx-test',
      mcpUrl: URL,
      harness: 'Codex',
    })
    const linkCall = stub.calls.find((c) => c.method === 'link')
    expect(linkCall?.payload).toMatchObject({
      server: { name: 'cdx-test', spec: { transport: 'http', url: URL } },
      agent: 'codex',
    })
    expect(outcome.installed).toBe(true)
  })

  test('installForAgent covers each of the 7 supported harnesses', async () => {
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    const harnesses = [
      { harness: 'Claude Code', agent: 'claude-code' },
      { harness: 'Codex', agent: 'codex' },
      { harness: 'Cursor', agent: 'cursor' },
      { harness: 'OpenCode', agent: 'opencode' },
      { harness: 'Antigravity', agent: 'antigravity' },
      { harness: 'VS Code', agent: 'vscode' },
      { harness: 'Zed', agent: 'zed' },
    ] as const
    for (const { harness, agent } of harnesses) {
      stub.reset()
      const outcome = await installForAgent({
        slug: `slug-${agent}`,
        mcpUrl: URL,
        harness,
      })
      const linkCall = stub.calls.find((c) => c.method === 'link')
      expect(linkCall?.payload).toMatchObject({
        server: { name: `slug-${agent}` },
        agent,
      })
      expect(outcome.installed).toBe(true)
    }
  })

  test('installForAgent surfaces the failure message when relink throws', async () => {
    const stub = createStubMcpManager()
    stub.link = async () => {
      throw new Error('nope')
    }
    setMcpManagerForTesting(stub)
    const outcome = await installForAgent({
      slug: 'boom',
      mcpUrl: URL,
      harness: 'Codex',
    })
    expect(outcome.installed).toBe(false)
    expect(outcome.message).toContain('Could not register endpoint')
    expect(outcome.message).toContain('nope')
  })

  test('uninstallForAgent uses the disconnect primitive with removeIfLast', async () => {
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    const outcome = await uninstallForAgent({
      slug: 'unslug',
      harness: 'Cursor',
    })
    const disc = stub.calls.find((c) => c.method === 'disconnect')
    expect(disc?.payload).toMatchObject({
      serverName: 'unslug',
      agent: 'cursor',
      removeIfLast: true,
    })
    expect(outcome.installed).toBe(false)
  })
})
