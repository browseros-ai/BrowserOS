import { describe, expect, it, mock } from 'bun:test'

let resolveInvitation: (value: { eligible: boolean }) => void = () => {}

mock.module('./client', () => ({
  apiClient: async () => ({
    getFeedbackInvitation: () =>
      new Promise<{ eligible: boolean }>((resolve) => {
        resolveInvitation = resolve
      }),
  }),
}))

const { useFeedbackInvitation } = await import('./feedback.hooks')

describe('feedback invitation query', () => {
  it('always refreshes the server authority when the card remounts', () => {
    const options = useFeedbackInvitation.getOptions()

    expect(options.staleTime).toBe(30_000)
    expect(options.refetchOnMount).toBe('always')
  })

  it('stamps the answer with when it was requested, not when it landed', async () => {
    const before = Date.now()
    const fetcher = useFeedbackInvitation.fetcher
    const pending = fetcher(
      // biome-ignore lint/suspicious/noExplicitAny: the fetcher ignores its context
      {} as any,
    )
    const requestedBy = Date.now()
    await new Promise((resolve) => setTimeout(resolve, 5))
    resolveInvitation({ eligible: true })
    const answer = await pending

    expect(answer.eligible).toBe(true)
    expect(answer.requestedAt).toBeGreaterThanOrEqual(before)
    expect(answer.requestedAt).toBeLessThanOrEqual(requestedBy)
  })
})
