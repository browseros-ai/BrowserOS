import { describe, expect, it } from 'bun:test'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import { buildAgentFilesystemToolSet } from '../../src/agent/ai-sdk-agent'
import type { ResolvedAgentConfig } from '../../src/agent/types'

function agentConfig(
  overrides: Partial<ResolvedAgentConfig> = {},
): ResolvedAgentConfig {
  return {
    conversationId: crypto.randomUUID(),
    provider: LLM_PROVIDERS.OPENAI,
    model: 'test-model',
    ...overrides,
  }
}

describe('buildAgentFilesystemToolSet', () => {
  it('includes only generated-output read access when no workspace is selected', () => {
    const tools = buildAgentFilesystemToolSet(agentConfig())
    expect(Object.keys(tools)).toEqual(['filesystem_read'])
  })

  it('includes filesystem tools when a workspace is selected', () => {
    const tools = buildAgentFilesystemToolSet(
      agentConfig({ workingDir: '/tmp/browseros-workspace' }),
    )
    expect(Object.keys(tools).sort()).toEqual([
      'filesystem_bash',
      'filesystem_edit',
      'filesystem_find',
      'filesystem_grep',
      'filesystem_ls',
      'filesystem_read',
      'filesystem_write',
    ])
  })

  it('exposes only output-file reads in chat mode even when a workspace is selected', () => {
    const tools = buildAgentFilesystemToolSet(
      agentConfig({ chatMode: true, workingDir: '/tmp/browseros-workspace' }),
    )
    expect(Object.keys(tools)).toEqual(['filesystem_read'])
  })

  it('omits filesystem tools for ACP providers', () => {
    const tools = buildAgentFilesystemToolSet(
      agentConfig({
        provider: LLM_PROVIDERS.CODEX,
        workingDir: '/tmp/browseros-workspace',
      }),
    )
    expect(Object.keys(tools)).toEqual([])
  })
})
