/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { type AcpSessionRecord, createFileSessionStore } from 'acpx/runtime'
import { buildAcpAgentPolicy } from '../../../../src/lib/agents/acp/acp-agent-policy'
import type { AcpAgentDefinition } from '../../../../src/lib/agents/agent-types'
import { BROWSEROS_TOOL_LEASE_HEADER } from '../../../../src/lib/browser-tool-lease'

const SKILL = [
  '---',
  'name: browseros',
  'description: BrowserOS browser skill',
  '---',
  'Use BrowserOS for every browser task.',
  '',
].join('\n')

const temporaryDirectories: string[] = []

afterEach(async () => {
  await Promise.all(
    temporaryDirectories
      .splice(0)
      .map((path) => rm(path, { recursive: true, force: true })),
  )
})

async function createResourcesDir(): Promise<string> {
  const resourcesDir = await mkdtemp(join(tmpdir(), 'acp-policy-'))
  temporaryDirectories.push(resourcesDir)
  const skillDir = join(resourcesDir, 'skills', 'browseros')
  await mkdir(skillDir, { recursive: true })
  await writeFile(join(skillDir, 'SKILL.md'), SKILL)
  return resourcesDir
}

function agent(
  type: AcpAgentDefinition['type'],
  patch: Partial<AcpAgentDefinition> = {},
): AcpAgentDefinition {
  return {
    id: `${type}-agent-id`,
    name: type === 'claude' ? 'Claude Code' : 'Codex',
    type,
    createdAt: 1,
    updatedAt: 1,
    ...patch,
  }
}

function agentArgv(
  policy: Awaited<ReturnType<typeof buildAcpAgentPolicy>>,
  type: AcpAgentDefinition['type'],
): string[] {
  const override = policy.agentRegistryOverrides[type]
  if (!Array.isArray(override)) throw new Error(`Expected ${type} argv`)
  return override
}

describe('buildAcpAgentPolicy', () => {
  it('builds a Claude session with BrowserOS MCP and appended skill guidance', async () => {
    const resourcesDir = await createResourcesDir()
    const policy = await buildAcpAgentPolicy({
      agent: agent('claude', {
        modelId: 'claude-opus-4-1',
        workingDirectory: '/work/project',
      }),
      conversationId: 'conversation-1',
      serverPort: 9001,
      browserToolLeaseToken: 'lease-1',
      readOnly: true,
      resourcesDir,
      browserosDir: '/state/browseros',
      browserContext: {
        windowId: 42,
        enabledMcpServers: ['Slack'],
        customMcpServers: [
          { name: 'github', url: 'https://mcp.example.com/github' },
        ],
      },
    })

    expect(policy.adapter).toBe('claude')
    expect(policy.cwd).toBe('/work/project')
    expect(policy.sessionKey).toBe('acp:claude-agent-id:conversation-1')
    expect(agentArgv(policy, 'claude')).toContain(
      '@agentclientprotocol/claude-agent-acp@^0.31.0',
    )
    expect(policy.mcpServers.map((server) => server.name)).toEqual([
      'browseros',
      'github',
    ])
    expect(policy.mcpServers[0]).toEqual({
      type: 'http',
      name: 'browseros',
      url: 'http://127.0.0.1:9001/mcp?read_only=1',
      headers: {
        [BROWSEROS_TOOL_LEASE_HEADER]: 'lease-1',
      },
    })
    expect(policy.sessionOptions).toEqual({
      model: 'claude-opus-4-1',
      systemPrompt: { append: SKILL },
    })
    expect(policy.fullAccessModeCandidates).toEqual(['bypassPermissions'])
  })

  it('configures Codex without a copied home and disables competing browser plugins', async () => {
    const resourcesDir = await createResourcesDir()
    const policy = await buildAcpAgentPolicy({
      agent: agent('codex', {
        modelId: 'gpt-5.4',
        reasoningEffort: 'high',
      }),
      conversationId: 'conversation-2',
      serverPort: 9002,
      browserToolLeaseToken: 'lease-2',
      readOnly: false,
      resourcesDir,
      browserosDir: '/state/browseros',
      browserContext: {
        customMcpServers: [
          { name: 'browseros', url: 'https://wrong.example.com/mcp' },
        ],
      },
    })

    const codexArgv = agentArgv(policy, 'codex')
    const store = createFileSessionStore({ stateDir: resourcesDir })
    const timestamp = new Date(0).toISOString()
    const record: AcpSessionRecord = {
      schema: 'acpx.session.v1',
      acpxRecordId: 'codex-record',
      acpSessionId: 'codex-session',
      agentCommand: codexArgv.join(' '),
      agentArgv: codexArgv,
      cwd: policy.cwd,
      createdAt: timestamp,
      lastUsedAt: timestamp,
      lastSeq: 0,
      eventLog: {
        active_path: 'events.jsonl',
        segment_count: 0,
        max_segment_bytes: 1024,
        max_segments: 1,
      },
      messages: [],
      updated_at: timestamp,
      cumulative_token_usage: {},
      request_token_usage: {},
      ...(policy.sessionOptions.env
        ? {
            acpx: {
              session_options: { env: policy.sessionOptions.env },
            },
          }
        : {}),
    }

    await store.save(record)
    expect(await store.load(record.acpxRecordId)).toBeDefined()
    expect(policy.sessionOptions).toEqual({})
    const renderedArgv = codexArgv.join('\n')
    expect(renderedArgv).not.toContain('CODEX_HOME')
    expect(renderedArgv).toContain('CODEX_CONFIG=')
    expect(renderedArgv).toContain('INITIAL_AGENT_MODE=agent-full-access')
    expect(renderedArgv).toContain(
      '"developer_instructions":"---\\nname: browseros',
    )
    expect(renderedArgv).toContain('"model":"gpt-5.4"')
    expect(renderedArgv).toContain('"model_reasoning_effort":"high"')
    expect(renderedArgv).toContain('"browser@openai-bundled":{"enabled":false}')
    expect(policy.mcpServers.map((server) => server.name)).toEqual([
      'browseros',
    ])
    expect(policy.fullAccessModeCandidates).toEqual([
      'agent-full-access',
      'full-access',
    ])
  })

  it('builds a custom agent policy from stored config', async () => {
    const resourcesDir = await createResourcesDir()
    const policy = await buildAcpAgentPolicy({
      agent: agent('custom', {
        id: 'custom-1',
        name: 'My Agent',
        modelId: 'gpt-x',
        customConfig: {
          command: 'npx -y @scope/my-agent-acp --stdio',
          env: { MY_AGENT_KEY: 'secret' },
          fullAccessModes: ['bypass'],
          systemPromptAppend: 'Extra instructions.',
        },
      }),
      conversationId: 'conversation-1',
      serverPort: 9001,
      browserToolLeaseToken: 'lease-3',
      readOnly: false,
      resourcesDir,
      browserosDir: '/state/browseros',
    })

    // Per-agent registry id, never a shared 'custom' key.
    expect(policy.adapter).toBe('custom:custom-1')
    const override = policy.agentRegistryOverrides['custom:custom-1']
    if (!Array.isArray(override)) throw new Error('Expected custom argv')
    expect(override.join(' ')).toContain('@scope/my-agent-acp')
    // Custom env rides at the process-launch boundary.
    expect(override.join(' ')).toContain('MY_AGENT_KEY=secret')
    expect(policy.sessionOptions).toEqual({
      model: 'gpt-x',
      systemPrompt: { append: 'Extra instructions.' },
    })
    expect(policy.fullAccessModeCandidates).toEqual(['bypass'])
    // BrowserOS MCP is injected regardless of agent type.
    expect(policy.mcpServers[0]?.name).toBe('browseros')
  })

  it('gives a custom agent no full-access modes when none are configured', async () => {
    const resourcesDir = await createResourcesDir()
    const policy = await buildAcpAgentPolicy({
      agent: agent('custom', {
        id: 'custom-2',
        name: 'Bare Agent',
        customConfig: { command: 'bare-agent' },
      }),
      conversationId: 'conversation-1',
      serverPort: 9001,
      browserToolLeaseToken: 'lease-4',
      readOnly: false,
      resourcesDir,
      browserosDir: '/state/browseros',
    })

    expect(policy.fullAccessModeCandidates).toEqual([])
    expect(policy.sessionOptions).toEqual({})
  })
})
