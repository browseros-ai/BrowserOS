import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'
import {
  buildSidepanelChatTargets,
  clearSidepanelChatTargetSelectionForAgent,
  persistSidepanelChatTargetSelection,
  resolveSidepanelChatTarget,
  type SidepanelChatTargetSelection,
} from './sidepanel-chat-targets'

const provider: LlmProviderConfig = {
  id: 'browseros',
  type: 'browseros',
  name: 'BrowserOS',
  modelId: 'browseros-auto',
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

  it('resolves a persisted ACP selection', () => {
    expect(
      resolveSidepanelChatTarget({
        targets,
        defaultProviderId: provider.id,
        selection: { kind: 'acp', id: agent.id },
      }),
    ).toMatchObject({ kind: 'acp', id: agent.id })
  })

  it('falls back to the default provider for a stale selection', () => {
    expect(
      resolveSidepanelChatTarget({
        targets,
        defaultProviderId: provider.id,
        selection: { kind: 'acp', id: 'deleted-agent' },
      }),
    ).toMatchObject({ kind: 'llm', id: provider.id })
  })
})

describe('target selection storage', () => {
  it('persists only target identity', async () => {
    const store = createSelectionStore()
    const target = buildSidepanelChatTargets({
      providers: [provider],
      agents: [agent],
    })[1]

    await persistSidepanelChatTargetSelection(target, store)

    expect(await store.getValue()).toEqual({ kind: 'acp', id: agent.id })
  })

  it('clears a selection when its agent is deleted', async () => {
    const store = createSelectionStore({ kind: 'acp', id: agent.id })

    await clearSidepanelChatTargetSelectionForAgent(agent.id, store)

    expect(await store.getValue()).toBeNull()
  })
})

function createSelectionStore(
  initial: SidepanelChatTargetSelection | null = null,
) {
  let value = initial
  return {
    getValue: async () => value,
    setValue: async (next: SidepanelChatTargetSelection | null) => {
      value = next
    },
    watch: () => () => {},
  }
}
