import { describe, expect, it } from 'bun:test'
import {
  isAgentCommandPath,
  isAgentConversationPath,
  shouldHideFocusGrid,
} from './route-utils'

describe('route-utils', () => {
  it('correctly identifies agent command center routes', () => {
    expect(isAgentCommandPath('/home')).toBe(true)
    expect(isAgentCommandPath('/home/agents/main')).toBe(true)
    expect(isAgentConversationPath('/home')).toBe(false)
    expect(isAgentConversationPath('/home/agents/main')).toBe(true)
  })

  it('hides the focus grid on full-screen routes', () => {
    expect(shouldHideFocusGrid('/home')).toBe(true)
    expect(shouldHideFocusGrid('/home/agents/main')).toBe(true)
    expect(shouldHideFocusGrid('/home/chat')).toBe(true)
    expect(shouldHideFocusGrid('/home/skills')).toBe(true)
    expect(shouldHideFocusGrid('/home/personalize')).toBe(false)
  })
})
