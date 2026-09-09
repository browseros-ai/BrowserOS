import { describe, expect, it, mock } from 'bun:test'
import {
  resolveRestoredChatTarget,
  restoreServerConversation,
} from './chat-session-restore'
import type { SidepanelChatTarget } from './sidepanel-chat-targets'

describe('restoreServerConversation', () => {
  it('waits for target selection before enabling a restored conversation', async () => {
    const steps: string[] = []
    await restoreServerConversation({
      conversationId: 'c1',
      fetchConversation: async () => ({ id: 'c1', messages: [] }),
      isCancelled: () => false,
      onRestore: async () => {
        steps.push('select agent')
        await Promise.resolve()
        steps.push('install transcript')
      },
      onError: () => {},
      onSettled: () => {
        steps.push('enable composer')
      },
    })
    expect(steps).toEqual([
      'select agent',
      'install transcript',
      'enable composer',
    ])
  })

  it('reports a missing row so the composer cannot send into an unrelated chat', async () => {
    const onMissing = mock(() => {})
    await restoreServerConversation({
      conversationId: 'deleted',
      fetchConversation: async () => null,
      isCancelled: () => false,
      onRestore: () => {
        throw new Error('must not restore')
      },
      onMissing,
      onError: () => {},
      onSettled: () => {},
    })
    expect(onMissing).toHaveBeenCalledTimes(1)
  })

  it('does not settle a view cancelled while selecting its target', async () => {
    let cancelled = false
    const onSettled = mock(() => {})
    await restoreServerConversation({
      conversationId: 'c1',
      fetchConversation: async () => ({ id: 'c1', messages: [] }),
      isCancelled: () => cancelled,
      onRestore: async () => {
        await Promise.resolve()
        cancelled = true
      },
      onError: () => {},
      onSettled,
    })
    expect(onSettled).not.toHaveBeenCalled()
  })
  it('applies the restored conversation and settles', async () => {
    const onRestore = mock(() => {})
    const onError = mock(() => {})
    const onSettled = mock(() => {})

    await restoreServerConversation({
      conversationId: 'c1',
      fetchConversation: async () => ({ id: 'c1', messages: [] }),
      isCancelled: () => false,
      onRestore,
      onError,
      onSettled,
    })

    expect(onRestore).toHaveBeenCalledWith({ id: 'c1', messages: [] })
    expect(onError).not.toHaveBeenCalled()
    expect(onSettled).toHaveBeenCalledTimes(1)
  })

  it('settles without restoring when the conversation is missing', async () => {
    const onRestore = mock(() => {})
    const onSettled = mock(() => {})

    await restoreServerConversation({
      conversationId: 'c1',
      fetchConversation: async () => null,
      isCancelled: () => false,
      onRestore,
      onError: mock(() => {}),
      onSettled,
    })

    expect(onRestore).not.toHaveBeenCalled()
    expect(onSettled).toHaveBeenCalledTimes(1)
  })

  it('ignores a stale response when cancelled mid-flight', async () => {
    const onRestore = mock(() => {})
    const onError = mock(() => {})
    const onSettled = mock(() => {})

    await restoreServerConversation({
      conversationId: 'c1',
      fetchConversation: async () => ({ id: 'c1', messages: [] }),
      isCancelled: () => true,
      onRestore,
      onError,
      onSettled,
    })

    expect(onRestore).not.toHaveBeenCalled()
    expect(onError).not.toHaveBeenCalled()
    expect(onSettled).not.toHaveBeenCalled()
  })

  it('reports the error but still settles on failure', async () => {
    const onError = mock(() => {})
    const onSettled = mock(() => {})
    const failure = new Error('network down')

    await restoreServerConversation({
      conversationId: 'c1',
      fetchConversation: async () => {
        throw failure
      },
      isCancelled: () => false,
      onRestore: mock(() => {}),
      onError,
      onSettled,
    })

    expect(onError).toHaveBeenCalledWith(failure)
    expect(onSettled).toHaveBeenCalledTimes(1)
  })

  it('neither reports nor settles when cancelled during a failure', async () => {
    const onError = mock(() => {})
    const onSettled = mock(() => {})

    await restoreServerConversation({
      conversationId: 'c1',
      fetchConversation: async () => {
        throw new Error('boom')
      },
      isCancelled: () => true,
      onRestore: mock(() => {}),
      onError,
      onSettled,
    })

    expect(onError).not.toHaveBeenCalled()
    expect(onSettled).not.toHaveBeenCalled()
  })
})

describe('conversation target ownership', () => {
  const llm = { kind: 'llm', id: 'openai' } as SidepanelChatTarget
  const codex = {
    kind: 'acp',
    id: 'codex',
    agentId: 'codex',
    agentType: 'codex',
  } as SidepanelChatTarget
  const otherCodex = {
    kind: 'acp',
    id: 'other',
    agentId: 'other',
    agentType: 'codex',
  } as SidepanelChatTarget
  const targets = [llm, codex, otherCodex]
  it('resumes the exact ACP agent even if another agent is selected', () => {
    expect(
      resolveRestoredChatTarget(
        { id: 'c1', messages: [], targetType: 'codex', agentId: 'codex' },
        targets,
        otherCodex,
      ),
    ).toBe(codex)
  })
  it('never substitutes another agent for a removed one', () => {
    expect(
      resolveRestoredChatTarget(
        { id: 'c1', messages: [], targetType: 'codex', agentId: 'removed' },
        targets,
        codex,
      ),
    ).toBeUndefined()
  })
  it('restores native history using an LLM instead of the currently selected ACP agent', () => {
    expect(
      resolveRestoredChatTarget(
        { id: 'c1', messages: [], targetType: 'browseros' },
        targets,
        codex,
      ),
    ).toBe(llm)
    expect(
      resolveRestoredChatTarget(
        { id: 'c1', messages: [], targetType: 'browseros' },
        targets,
        llm,
      ),
    ).toBe(llm)
  })
})
