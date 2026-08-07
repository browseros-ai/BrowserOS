import { describe, expect, it, mock } from 'bun:test'
import { buildChatErrorProps } from './Chat.helpers'

describe('buildChatErrorProps', () => {
  it('passes a retry handler to chat errors', () => {
    const retryLastTurn = mock(async () => {})
    const error = new Error('Failed to fetch')

    const props = buildChatErrorProps({
      chatError: error,
      selectedProvider: { type: 'browseros' },
      retryLastTurn,
    })

    expect(props).toMatchObject({
      error,
      providerType: 'browseros',
    })
    expect(props?.onRetry).toBeDefined()

    props?.onRetry?.()

    expect(retryLastTurn).toHaveBeenCalledTimes(1)
  })

  it('returns no props when there is no chat error', () => {
    expect(
      buildChatErrorProps({
        chatError: null,
        retryLastTurn: () => {},
      }),
    ).toBeNull()
  })
})
