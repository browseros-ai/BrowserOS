/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, test } from 'bun:test'
import { setMcpManagerForTesting } from '../../src/lib/mcp-manager'
import type { NewAgentValues } from '../../src/routes/agents/schemas'
import * as agents from '../../src/routes/agents/service'
import {
  installForAgent,
  uninstallForAgent,
} from '../../src/services/harness-install'
import { createStubMcpManager } from '../_helpers/stub-mcp-manager'
import { withTempBrowserosDir } from '../_helpers/temp-browseros-dir'

function makeInput(): NewAgentValues {
  return {
    name: 'Install Smoke',
    harness: 'Claude Desktop',
    loginMode: 'profile',
    selectedSites: [],
    approvals: {
      submit: 'Ask',
      payment: 'Block',
      delete: 'Ask',
      upload: 'Ask',
      navigate: 'Auto',
      input: 'Auto',
    },
    aclRuleIds: [],
    customAclRules: [],
  }
}

describe('harness install service', () => {
  test('installForAgent on Claude Desktop links the slug under claude-desktop', async () => {
    await withTempBrowserosDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const created = await agents.create(makeInput())
      const addCall = stub.calls.find((c) => c.method === 'add')
      const linkCall = stub.calls.find((c) => c.method === 'link')
      expect(addCall?.payload).toMatchObject({
        name: created.slug,
        spec: { transport: 'http', url: created.mcpUrl },
      })
      expect(linkCall?.payload).toMatchObject({
        serverName: created.slug,
        agent: 'claude-desktop',
      })
      expect(created.harnessInstall.installed).toBe(true)
      expect(created.harnessInstall.message).toContain('Claude Desktop')
    })
  })

  test('installForAgent on Codex uses stdio + mcp-remote', async () => {
    await withTempBrowserosDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const outcome = await installForAgent({
        slug: 'cdx-test',
        mcpUrl: 'http://127.0.0.1:9200/mcp/cdx-test',
        harness: 'Codex',
      })
      const addCall = stub.calls.find((c) => c.method === 'add')
      expect(addCall?.payload).toMatchObject({
        name: 'cdx-test',
        spec: {
          transport: 'stdio',
          command: 'npx',
          args: ['mcp-remote', 'http://127.0.0.1:9200/mcp/cdx-test'],
        },
      })
      const linkCall = stub.calls.find((c) => c.method === 'link')
      expect(linkCall?.payload).toMatchObject({ agent: 'codex' })
      expect(outcome.installed).toBe(true)
    })
  })

  test('Hermes + OpenClaw short-circuit as a no-op success (no manager calls)', async () => {
    await withTempBrowserosDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      for (const harness of ['Hermes', 'OpenClaw'] as const) {
        const outcome = await installForAgent({
          slug: 'x',
          mcpUrl: 'http://127.0.0.1:9200/mcp/x',
          harness,
        })
        expect(outcome.installed).toBe(true)
        expect(outcome.message.toLowerCase()).toContain('browseros')
      }
      expect(stub.calls).toEqual([])
    })
  })

  test('uninstallForAgent unlinks and drops the manifest entry', async () => {
    await withTempBrowserosDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      await uninstallForAgent({ slug: 'gone-slug', harness: 'Claude Desktop' })
      const methods = stub.calls.map((c) => c.method)
      expect(methods).toContain('unlink')
      expect(methods).toContain('remove')
    })
  })

  test('install failure does not throw; outcome carries the message', async () => {
    await withTempBrowserosDir(async () => {
      const stub = createStubMcpManager()
      // Inject a custom failing manager.
      stub.add = async () => {
        throw new Error('disk full')
      }
      setMcpManagerForTesting(stub)
      const outcome = await installForAgent({
        slug: 'broken',
        mcpUrl: 'http://127.0.0.1:9200/mcp/broken',
        harness: 'Claude Desktop',
      })
      expect(outcome.installed).toBe(false)
      expect(outcome.message).toContain('Claude Desktop')
      expect(outcome.message).toContain('disk full')
    })
  })
})
