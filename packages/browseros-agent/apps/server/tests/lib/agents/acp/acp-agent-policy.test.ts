/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { buildAcpAgentPolicy } from '../../../../src/lib/agents/acp/acp-agent-policy'
import type { AcpAgentDefinition } from '../../../../src/lib/agents/agent-types'

const SKILL = [
  '---',
  'name: browserclaw',
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
  const skillDir = join(resourcesDir, 'skills', 'browserclaw')
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
    expect(policy.agentRegistryOverrides.claude).toContain(
      '@agentclientprotocol/claude-agent-acp@^0.31.0',
    )
    expect(policy.mcpServers.map((server) => server.name)).toEqual([
      'browseros',
      'github',
    ])
    expect(policy.mcpServers[0]).toEqual({
      type: 'http',
      name: 'browseros',
      url: 'http://127.0.0.1:9001/mcp',
      headers: {
        'X-BrowserOS-Scope-Id': 'conversation-1',
        'X-BrowserOS-Default-Window-Id': '42',
        'X-BrowserOS-Managed-Mcp-Servers': 'Slack',
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
      resourcesDir,
      browserosDir: '/state/browseros',
      browserContext: {
        customMcpServers: [
          { name: 'browseros', url: 'https://wrong.example.com/mcp' },
        ],
      },
    })

    expect(policy.sessionOptions.env?.CODEX_HOME).toBeUndefined()
    expect(policy.sessionOptions.env?.INITIAL_AGENT_MODE).toBe(
      'agent-full-access',
    )
    const config = JSON.parse(policy.sessionOptions.env?.CODEX_CONFIG ?? '{}')
    expect(config).toEqual({
      developer_instructions: SKILL,
      model: 'gpt-5.4',
      model_reasoning_effort: 'high',
      plugins: {
        'browser@openai-bundled': { enabled: false },
        'chrome@openai-bundled': { enabled: false },
        'computer-use@openai-bundled': { enabled: false },
      },
    })
    expect(policy.mcpServers.map((server) => server.name)).toEqual([
      'browseros',
    ])
    expect(policy.fullAccessModeCandidates).toEqual([
      'agent-full-access',
      'full-access',
    ])
  })
})
