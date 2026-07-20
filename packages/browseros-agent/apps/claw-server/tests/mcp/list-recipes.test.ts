/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * End-to-end coverage for the list_recipes MCP tool. Locks the
 * schema surface, argument acceptance (URL vs bare hostname), and
 * the wire shape returned to callers.
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { mkdirSync, writeFileSync } from 'node:fs'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'

const realFramework = await import('@browseros/browser-mcp/tools/framework')
mock.module('@browseros/browser-mcp/tools/framework', () => ({
  ...realFramework,
  executeTool: async (
    _tool: { name: string },
    _args: Record<string, unknown>,
  ) => ({ isError: false, content: [{ type: 'text', text: 'ok' }] }),
}))

const { ownershipStore } = await import('../../src/domain/ownership')
const { setBrowserSession } = await import('../../src/lib/browser-session')
const { identityService } = await import('../../src/lib/mcp-session')
const { resetTabGroupEffectsForTesting } = await import(
  '../../src/mcp/effects/tab-groups'
)
const { resetSingleMcpInstanceForTesting } = await import(
  '../../src/mcp/single-server'
)
const { resetAuditDbForTesting, setAuditDbForTesting } = await import(
  '../../src/modules/db/db'
)
const { agentRecipesDirFor, sharedRecipesDirFor } = await import(
  '../../src/services/recipes'
)
const { createServer } = await import('../../src/server')
const { withTempBrowserClawDir } = await import(
  '../_helpers/temp-browserclaw-dir'
)

const app = createServer()

async function connect() {
  const transport = new StreamableHTTPClientTransport(
    new URL('http://localhost/mcp'),
    {
      fetch: ((input: RequestInfo | URL, init?: RequestInit) =>
        app.fetch(new Request(input, init))) as typeof fetch,
    },
  )
  const client = new Client(
    { name: 'claude-code', version: '1.0.0' },
    { capabilities: {} },
  )
  await client.connect(transport)
  const sessionId = transport.sessionId
  if (!sessionId) throw new Error('missing session id')
  const identity = identityService.getIdentity(sessionId)
  if (!identity) throw new Error('missing identity')
  return { client, identity }
}

function textOf(result: { content?: unknown }): string {
  return (
    (result.content as Array<{ text?: string }> | undefined)?.[0]?.text ?? ''
  )
}

function seedRecipe(dir: string, name: string, body = '# stub'): void {
  mkdirSync(dir, { recursive: true })
  writeFileSync(`${dir}/${name}`, body)
}

describe('list_recipes', () => {
  beforeEach(() => {
    setAuditDbForTesting()
    resetSingleMcpInstanceForTesting()
    resetTabGroupEffectsForTesting()
    identityService.clear()
    ownershipStore.clear()
    setBrowserSession(null)
  })

  afterEach(() => {
    resetSingleMcpInstanceForTesting()
    resetTabGroupEffectsForTesting()
    identityService.clear()
    ownershipStore.clear()
    setBrowserSession(null)
    resetAuditDbForTesting()
  })

  it('registers a read-only tool with the expected schema', async () => {
    await withTempBrowserClawDir(async () => {
      const { client } = await connect()
      const tool = (await client.listTools()).tools.find(
        (candidate) => candidate.name === 'list_recipes',
      )
      expect(tool).toMatchObject({
        name: 'list_recipes',
        annotations: {
          title: 'List recipes',
          readOnlyHint: true,
          destructiveHint: false,
          idempotentHint: true,
        },
        inputSchema: {
          type: 'object',
          properties: {
            host: { type: 'string', minLength: 1, maxLength: 256 },
          },
          required: ['host'],
        },
      })
      expect(tool?.description).toContain('context caching')
      await client.close()
    })
  })

  it('returns an empty listing with a cold-nudge summary for a fresh host', async () => {
    await withTempBrowserClawDir(async () => {
      const { client } = await connect()
      const result = await client.callTool({
        name: 'list_recipes',
        arguments: { host: 'linkedin.com' },
      })
      expect(result.isError).toBeFalsy()
      const structured = result.structuredContent as {
        host: string
        files: unknown[]
        workspace_dir: string
      }
      expect(structured.host).toBe('linkedin.com')
      expect(structured.files).toEqual([])
      expect(structured.workspace_dir).toContain('recipes/shared/linkedin.com')
      expect(textOf(result)).toContain('none yet')
      await client.close()
    })
  })

  it('surfaces shared + agent overlay files when they exist', async () => {
    await withTempBrowserClawDir(async () => {
      const { client, identity } = await connect()
      seedRecipe(sharedRecipesDirFor('linkedin.com'), 'invitation.md')
      seedRecipe(agentRecipesDirFor(identity.slug, 'linkedin.com'), 'mine.md')

      const result = await client.callTool({
        name: 'list_recipes',
        arguments: { host: 'linkedin.com' },
      })
      const structured = result.structuredContent as {
        files: Array<{ name: string; source: string }>
      }
      expect(structured.files.map((f) => `${f.name}:${f.source}`)).toEqual([
        'invitation.md:shared',
        'mine.md:agent',
      ])
      await client.close()
    })
  })

  it('accepts a full http(s) URL and buckets it correctly', async () => {
    await withTempBrowserClawDir(async () => {
      const { client } = await connect()
      seedRecipe(sharedRecipesDirFor('docs.google.com'), 'docs.md')

      const result = await client.callTool({
        name: 'list_recipes',
        arguments: {
          host: 'https://docs.google.com/document/d/abc',
        },
      })
      const structured = result.structuredContent as {
        host: string
        files: Array<{ name: string }>
      }
      expect(structured.host).toBe('docs.google.com')
      expect(structured.files.map((f) => f.name)).toEqual(['docs.md'])
      await client.close()
    })
  })

  it('rejects a host argument that is neither URL nor plausible hostname', async () => {
    await withTempBrowserClawDir(async () => {
      const { client } = await connect()
      const result = await client.callTool({
        name: 'list_recipes',
        arguments: { host: 'not a host' },
      })
      expect(result.isError).toBe(true)
      expect(textOf(result)).toContain('not a routable host')
      await client.close()
    })
  })
})
