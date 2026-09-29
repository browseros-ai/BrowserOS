import { describe, expect, it } from 'bun:test'
import { useFeedbackInvitation } from './feedback.hooks'

describe('feedback invitation query', () => {
  it('always refreshes the server authority when the card remounts', () => {
    const options = useFeedbackInvitation.getOptions()

    expect(options.staleTime).toBe(30_000)
    expect(options.refetchOnMount).toBe('always')
  })
})
