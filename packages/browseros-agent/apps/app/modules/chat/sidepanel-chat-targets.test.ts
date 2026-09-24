import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'
import {
  buildSidepanelChatTargets,
  resolveSidepanelChatTarget,
} from './sidepanel-chat-targets'

const provider: LlmProviderConfig = {
  id: 'openai-1',
  type: 'openai',
  name: 'OpenAI',
  modelId: 'gpt-5',
  supportsImages: true,
  contextWindow: 200000,
  temperature: 0.2,
  createdAt: 1,
  updatedAt: 1,
}

const agent: AcpAgent = {
  id: '00000000-0000-4000-8000-000000000001',
  name: 'Review Bot',
  type: 'codex',
  modelId: 'gpt-5.5',
  reasoningEffort: 'high',
  createdAt: 1,
  updatedAt: 1,
}

describe('buildSidepanelChatTargets', () => {
  it('combines model providers and persisted ACP agents', () => {
    const targets = buildSidepanelChatTargets({
      providers: [provider],
      agents: [agent],
    })

    expect(targets).toHaveLength(2)
    expect(targets[1]).toMatchObject({
      kind: 'acp',
      agentId: agent.id,
      agentType: 'codex',
      adapterName: 'Codex',
      modelId: 'gpt-5.5',
      reasoningEffort: 'high',
    })
  })

  it('uses agent defaults when model and reasoning are unset', () => {
    const targets = buildSidepanelChatTargets({
      providers: [],
      agents: [{ ...agent, modelId: undefined, reasoningEffort: undefined }],
    })

    expect(targets[0]).toMatchObject({
      modelId: 'default',
      modelLabel: 'Agent default',
      reasoningEffort: 'default',
    })
  })
})

describe('resolveSidepanelChatTarget', () => {
  const targets = buildSidepanelChatTargets({
    providers: [provider],
    agents: [agent],
  })

  it('resolves the named provider', () => {
    expect(
      resolveSidepanelChatTarget({ targets, defaultTargetId: provider.id }),
    ).toMatchObject({ kind: 'llm', id: provider.id })
  })

  it('honours a default id that names a coding agent', () => {
    // Both kinds are rows in one table, so the id is all that is stored. The
    // resolver matching across the whole list is what makes that enough.
    expect(
      resolveSidepanelChatTarget({ targets, defaultTargetId: agent.id }),
    ).toMatchObject({ kind: 'acp', id: agent.id })
  })

  it('falls back rather than resolving to nothing for a deleted id', () => {
    // Deleting a row takes the default with it, but a surface can hold the old
    // id until it refetches. Falling back keeps a target on screen meanwhile.
    expect(
      resolveSidepanelChatTarget({ targets, defaultTargetId: 'deleted-row' }),
    ).toMatchObject({ kind: 'llm', id: provider.id })
  })

  it('falls back to a coding agent when that is all there is', () => {
    // The fallback used to consider only LLM providers, which was invisible
    // while a built-in one was always present. Someone who connected just a
    // coding agent resolved to nothing.
    const agentOnly = buildSidepanelChatTargets({
      providers: [],
      agents: [agent],
    })

    expect(
      resolveSidepanelChatTarget({ targets: agentOnly, defaultTargetId: null }),
    ).toMatchObject({ kind: 'acp', id: agent.id })
  })

  it('resolves to nothing when nothing is connected', () => {
    expect(
      resolveSidepanelChatTarget({ targets: [], defaultTargetId: null }),
    ).toBeUndefined()
  })
})
