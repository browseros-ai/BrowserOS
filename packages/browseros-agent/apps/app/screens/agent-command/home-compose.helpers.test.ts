import { describe, expect, it } from 'bun:test'
import { resolveHomeLlmRoutingMode } from './home-compose.helpers'

describe('resolveHomeLlmRoutingMode', () => {
  it('waits for capability initialization before falling back', () => {
    expect(
      resolveHomeLlmRoutingMode({
        capabilitiesLoading: true,
        supportsInlineChat: false,
      }),
    ).toBe('wait')
  })

  it('uses inline chat when capability checks pass', () => {
    expect(
      resolveHomeLlmRoutingMode({
        capabilitiesLoading: false,
        supportsInlineChat: true,
      }),
    ).toBe('inline-chat')
  })

  it('falls back after capability checks finish unsupported', () => {
    expect(
      resolveHomeLlmRoutingMode({
        capabilitiesLoading: false,
        supportsInlineChat: false,
      }),
    ).toBe('sidepanel')
  })
})
