import { describe, expect, it } from 'bun:test'
import { panelTabIdFromUrl } from './panelTab'

describe('fixed contextual panel ownership', () => {
  it('reads the native path query independently of React hash routing', () => {
    expect(
      panelTabIdFromUrl(
        'chrome-extension://assistant/sidepanel.html?tabId=42#/chat?conversationId=other',
      ),
    ).toBe(42)
  })
  it('rejects absent and invalid tab ids instead of binding tab zero', () => {
    for (const value of [
      '',
      '?tabId=',
      '?tabId=-1',
      '?tabId=1.2',
      '?tabId=NaN',
    ]) {
      expect(
        panelTabIdFromUrl(
          `chrome-extension://assistant/sidepanel.html${value}`,
        ),
      ).toBeUndefined()
    }
  })
})
