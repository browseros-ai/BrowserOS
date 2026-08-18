import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'
import {
  buildSidepanelChatTargets,
  clearSidepanelChatTargetSelectionForAgent,
  persistSidepanelChatTargetSelection,
  resolveRepairedSelection,
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

describe('resolveRepairedSelection', () => {
  const targets = buildSidepanelChatTargets({
    providers: [provider],
    agents: [agent],
  })
  const llmTarget = targets[0]
  const acpTarget = targets[1]

  it('keeps an ACP selection while agents are still loading (not ready)', () => {
    // Regression guard: agents not settled yet, so the resolved target has
    // fallen back to the LLM provider. The stored ACP selection must survive.
    expect(
      resolveRepairedSelection({
        selection: { kind: 'acp', id: agent.id },
        resolvedTarget: llmTarget,
        ready: false,
      }),
    ).toEqual({ repair: false })
  })

  it('keeps a selection that matches the resolved target', () => {
    expect(
      resolveRepairedSelection({
        selection: { kind: 'acp', id: agent.id },
        resolvedTarget: acpTarget,
        ready: true,
      }),
    ).toEqual({ repair: false })
  })

  it('repairs a stale selection to the resolved fallback once ready', () => {
    expect(
      resolveRepairedSelection({
        selection: { kind: 'acp', id: 'deleted-agent' },
        resolvedTarget: llmTarget,
        ready: true,
      }),
    ).toEqual({ repair: true, selection: { kind: 'llm', id: provider.id } })
  })

  it('repairs to null when nothing resolves', () => {
    expect(
      resolveRepairedSelection({
        selection: { kind: 'llm', id: 'gone' },
        resolvedTarget: undefined,
        ready: true,
      }),
    ).toEqual({ repair: true, selection: null })
  })

  it('never repairs when there is no stored selection', () => {
    expect(
      resolveRepairedSelection({
        selection: null,
        resolvedTarget: llmTarget,
        ready: true,
      }),
    ).toEqual({ repair: false })
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
