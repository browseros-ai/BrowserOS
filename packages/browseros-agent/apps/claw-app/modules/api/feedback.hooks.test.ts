import { describe, expect, it, mock } from 'bun:test'
import { QueryClient } from '@tanstack/react-query'

const recordFeedbackInvite = mock(async (_request: unknown) => ({
  eligible: true,
  round: 2,
}))

mock.module('./client', () => ({
  apiClient: async () => ({ recordFeedbackInvite }),
}))

const { useFeedbackInvitation, useRecordFeedbackInvite } = await import(
  './feedback.hooks'
)

describe('feedback invitation query', () => {
  it('always refreshes the server authority when the card remounts', () => {
    const options = useFeedbackInvitation.getOptions()

    expect(options.staleTime).toBe(30_000)
    expect(options.refetchOnMount).toBe('always')
  })

  it('forwards the shown round through the outcome mutation', async () => {
    const request = { outcome: 'clicked' as const, round: 2 }
    const mutation = useRecordFeedbackInvite.getOptions().mutationFn
    if (!mutation) throw new Error('Missing outcome mutation')
    await mutation(request, {
      client: new QueryClient(),
      meta: undefined,
      mutationKey: undefined,
    })

    expect(recordFeedbackInvite).toHaveBeenCalledWith(request)
  })
})
