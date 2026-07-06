/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, test } from 'bun:test'
import { env } from '../../src/env'
import { setMcpManagerForTesting } from '../../src/lib/mcp-manager'
import type { NewAgentValues } from '../../src/routes/agents/schemas'
import * as agents from '../../src/routes/agents/service'
import {
  installForAgent,
  uninstallForAgent,
} from '../../src/services/harness-install'
import { createStubMcpManager } from '../_helpers/stub-mcp-manager'
import { withTempBrowserClawDir } from '../_helpers/temp-browserclaw-dir'

function makeInput(overrides: Partial<NewAgentValues> = {}): NewAgentValues {
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
    ...overrides,
  }
}

describe('harness install service', () => {
  test('installForAgent on Claude Desktop wraps the URL in npx mcp-remote (stdio-only parser)', async () => {
    // Claude Desktop's `claude_desktop_config.json` parser validates
    // stdio-shaped entries only, so the install path must write the
    // `npx mcp-remote <url>` shape. specFor sources this from the
    // agent-mcp-manager catalog via resolveAgentSurface.
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const created = await agents.create(makeInput())
      const addCall = stub.calls.find((c) => c.method === 'add')
      const linkCall = stub.calls.find((c) => c.method === 'link')
      expect(addCall?.payload).toMatchObject({
        name: created.slug,
        spec: {
          transport: 'stdio',
          command: 'npx',
          args: ['mcp-remote', created.mcpUrl],
        },
      })
      expect(linkCall?.payload).toMatchObject({
        serverName: created.slug,
        agent: 'claude-desktop',
      })
      expect(created.harnessInstall.installed).toBe(true)
      expect(created.harnessInstall.message).toContain('Claude Desktop')
    })
  })

  test('installForAgent on Codex writes a direct HTTP spec (http-capable since agent-mcp-manager 0.0.3)', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const outcome = await installForAgent({
        slug: 'cdx-test',
        mcpUrl: 'http://127.0.0.1:9200/mcp',
        harness: 'Codex',
      })
      const addCall = stub.calls.find((c) => c.method === 'add')
      expect(addCall?.payload).toMatchObject({
        name: 'cdx-test',
        spec: {
          transport: 'http',
          url: 'http://127.0.0.1:9200/mcp',
        },
      })
      const linkCall = stub.calls.find((c) => c.method === 'link')
      expect(linkCall?.payload).toMatchObject({ agent: 'codex' })
      expect(outcome.installed).toBe(true)
    })
  })

  test('Hermes + OpenClaw short-circuit as a no-op success (no manager calls)', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      for (const harness of ['Hermes', 'OpenClaw'] as const) {
        const outcome = await installForAgent({
          slug: 'x',
          mcpUrl: 'http://127.0.0.1:9200/mcp',
          harness,
        })
        expect(outcome.installed).toBe(true)
        expect(outcome.message.toLowerCase()).toContain('browseros')
      }
      expect(stub.calls).toEqual([])
    })
  })

  test('uninstallForAgent unlinks and drops the manifest entry', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      await uninstallForAgent({ slug: 'gone-slug', harness: 'Claude Desktop' })
      const methods = stub.calls.map((c) => c.method)
      expect(methods).toContain('unlink')
      expect(methods).toContain('remove')
    })
  })

  test('install failure does not throw; outcome carries the message', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      // Inject a custom failing manager.
      stub.add = async () => {
        throw new Error('disk full')
      }
      setMcpManagerForTesting(stub)
      const outcome = await installForAgent({
        slug: 'broken',
        mcpUrl: 'http://127.0.0.1:9200/mcp',
        harness: 'Claude Desktop',
      })
      expect(outcome.installed).toBe(false)
      expect(outcome.message).toContain('Claude Desktop')
      expect(outcome.message).toContain('disk full')
    })
  })

  test('update with a slug rotation re-links the new slug then unlinks the old one', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const created = await agents.create(makeInput({ name: 'Original Name' }))
      // Drop the create calls so the assertion below only sees the reconcile.
      stub.reset()
      await agents.update(created.id, makeInput({ name: 'Renamed Profile' }))
      const order = stub.calls.map((c) => ({
        method: c.method,
        name:
          (c.payload as { name?: string; serverName?: string }).name ??
          (c.payload as { serverName?: string }).serverName,
      }))
      const addIdx = order.findIndex(
        (o) => o.method === 'add' && o.name === 'renamed-profile',
      )
      const linkIdx = order.findIndex(
        (o) => o.method === 'link' && o.name === 'renamed-profile',
      )
      const unlinkIdx = order.findIndex(
        (o) => o.method === 'unlink' && o.name === 'original-name',
      )
      const removeIdx = order.findIndex(
        (o) => o.method === 'remove' && o.name === 'original-name',
      )
      expect(addIdx).toBeGreaterThanOrEqual(0)
      expect(linkIdx).toBeGreaterThan(addIdx)
      expect(unlinkIdx).toBeGreaterThan(linkIdx)
      expect(removeIdx).toBeGreaterThan(unlinkIdx)
    })
  })

  test('update with a harness change writes the new harness and unlinks the old one', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const created = await agents.create(
        makeInput({ name: 'Stable Name', harness: 'Claude Code' }),
      )
      stub.reset()
      await agents.update(
        created.id,
        makeInput({ name: 'Stable Name', harness: 'Cursor' }),
      )
      const linkCall = stub.calls.find((c) => c.method === 'link')
      const unlinkCall = stub.calls.find((c) => c.method === 'unlink')
      expect(linkCall?.payload).toMatchObject({ agent: 'cursor' })
      expect(unlinkCall?.payload).toMatchObject({ agent: 'claude-code' })
    })
  })

  test('update with no harness or slug change skips the reconcile entirely', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const created = await agents.create(makeInput({ name: 'Same' }))
      stub.reset()
      // Mutate something irrelevant to the harness link (approvals).
      await agents.update(created.id, {
        ...makeInput({ name: 'Same' }),
        approvals: {
          submit: 'Block',
          payment: 'Block',
          delete: 'Block',
          upload: 'Block',
          navigate: 'Block',
          input: 'Block',
        },
      })
      expect(stub.calls).toEqual([])
      void created
    })
  })

  test('update with only an MCP URL change re-links the existing slug', async () => {
    await withTempBrowserClawDir(async () => {
      const previousProxyPort = env.proxyPort
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const created = await agents.create(makeInput({ name: 'Same Url Drift' }))
      stub.reset()
      env.proxyPort = 9512
      stub.listLinks = async () => {
        stub.calls.push({
          method: 'listLinks',
          payload: { serverNames: [created.slug] },
        })
        return [
          {
            serverName: created.slug,
            agent: 'claude-desktop',
            configPath: '/tmp/stub-claude-desktop.json',
          },
        ]
      }
      try {
        await agents.update(created.id, makeInput({ name: 'Same Url Drift' }))
      } finally {
        env.proxyPort = previousProxyPort
      }
      expect(stub.calls.map((c) => c.method)).toEqual([
        'listLinks',
        'listServers',
        'add',
        'unlink',
        'link',
      ])
      const add = stub.calls.find((c) => c.method === 'add')
      expect(add?.payload).toMatchObject({
        name: created.slug,
        spec: {
          command: 'npx',
          args: ['mcp-remote', 'http://127.0.0.1:9512/mcp'],
        },
      })
      const link = stub.calls.find((c) => c.method === 'link')
      expect(link?.payload).toMatchObject({
        configPath: '/tmp/stub-claude-desktop.json',
      })
    })
  })

  test('installForAgent restores the previous managed link when replacement link fails', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      const previousSpec = {
        transport: 'stdio' as const,
        command: 'npx',
        args: ['mcp-remote', 'http://127.0.0.1:9200/mcp'],
      }
      stub.listLinks = async () => {
        stub.calls.push({
          method: 'listLinks',
          payload: { serverNames: ['existing'] },
        })
        return [
          {
            serverName: 'existing',
            agent: 'claude-desktop',
            configPath: '/tmp/stub-claude-desktop.json',
          },
        ]
      }
      stub.listServers = async () => {
        stub.calls.push({ method: 'listServers', payload: {} })
        return [
          {
            name: 'existing',
            spec: previousSpec,
            addedAt: '2026-07-02T00:00:00.000Z',
            links: {},
          },
        ]
      }
      let linkAttempts = 0
      stub.link = async (opts) => {
        stub.calls.push({ method: 'link', payload: opts })
        linkAttempts++
        if (linkAttempts === 1) throw new Error('write denied')
        return {
          serverName: opts.serverName,
          agent: opts.agent,
          configPath: opts.configPath ?? `/tmp/stub-${opts.agent}.json`,
          created: true,
        }
      }
      setMcpManagerForTesting(stub)

      const outcome = await installForAgent({
        slug: 'existing',
        mcpUrl: 'http://127.0.0.1:9512/mcp',
        harness: 'Claude Desktop',
      })

      expect(outcome.installed).toBe(false)
      expect(outcome.message).toContain('write denied')
      const addCalls = stub.calls.filter((c) => c.method === 'add')
      expect(addCalls).toHaveLength(2)
      expect(addCalls[0]?.payload).toMatchObject({
        name: 'existing',
        spec: {
          args: ['mcp-remote', 'http://127.0.0.1:9512/mcp'],
        },
      })
      expect(addCalls[1]?.payload).toMatchObject({
        name: 'existing',
        spec: previousSpec,
      })
      const linkCalls = stub.calls.filter((c) => c.method === 'link')
      expect(linkCalls).toHaveLength(2)
      expect(linkCalls[1]?.payload).toMatchObject({
        serverName: 'existing',
        agent: 'claude-desktop',
        configPath: '/tmp/stub-claude-desktop.json',
      })
    })
  })

  test('regenerateMcpUrl re-links the new slug and unlinks the old one', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const created = await agents.create(makeInput({ name: 'Rotate Me' }))
      stub.reset()
      const rotated = await agents.regenerateMcpUrl(created.id)
      expect(rotated).not.toBeNull()
      const linkCall = stub.calls.find((c) => c.method === 'link')
      const unlinkCall = stub.calls.find((c) => c.method === 'unlink')
      const linkedServerName = (
        linkCall?.payload as { serverName?: string } | undefined
      )?.serverName
      expect(linkedServerName).toMatch(/^rotate-me-[a-z0-9-]+$/)
      expect(linkedServerName).not.toBe(created.slug)
      expect(linkCall?.payload).toMatchObject({ agent: 'claude-desktop' })
      expect(unlinkCall?.payload).toMatchObject({
        serverName: created.slug,
        agent: 'claude-desktop',
      })
    })
  })
})
