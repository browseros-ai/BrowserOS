import { beforeEach, describe, expect, it, mock } from 'bun:test'
import { createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'

let localRows: Array<{
  id: string
  lastMessagedAt: number
  lastUserMessage: string
}>

mock.module('@/modules/conversations/conversations.hooks', () => ({
  useServerConversations: () => ({ data: localRows }),
  useDeleteServerConversation: () => ({ mutate: () => {} }),
}))
mock.module('@/modules/chat/chat-session-context', () => ({
  useChatSessionContext: () => ({ conversationId: 'active' }),
}))

const { ChatHistory } = (await import('./ChatHistory')) as { ChatHistory: FC }

beforeEach(() => {
  localRows = []
})

// The real ConversationList links to each conversation, so it needs a router.
// The previous version of this test never hit that, because it mocked the list
// away and only checked which sections rendered.
function render() {
  return renderToStaticMarkup(
    createElement(MemoryRouter, null, createElement(ChatHistory)),
  )
}

describe('ChatHistory', () => {
  it('lists the conversations stored on this machine', () => {
    localRows = [
      { id: 'a', lastMessagedAt: Date.now(), lastUserMessage: 'hello there' },
    ]

    expect(render()).toContain('hello there')
  })

  it('says so when this machine has none, rather than rendering nothing', () => {
    expect(render()).toContain('No conversations on this device yet')
  })

  // There is one source now. The account list, the heading that introduced it
  // and the deduplication between the two are all gone with sign-in.
  it('shows no account section', () => {
    localRows = [
      { id: 'a', lastMessagedAt: Date.now(), lastUserMessage: 'hello' },
    ]
    const html = render()

    expect(html).not.toContain('account')
    expect(html).not.toContain('Account')
  })

  it('keeps one scroll container', () => {
    expect((render().match(/<main/g) ?? []).length).toBe(1)
  })
})
