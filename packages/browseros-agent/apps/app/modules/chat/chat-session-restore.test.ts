import { describe, expect, it, mock } from 'bun:test'
import { restoreServerConversation } from './chat-session-restore'

describe('restoreServerConversation', () => {
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
