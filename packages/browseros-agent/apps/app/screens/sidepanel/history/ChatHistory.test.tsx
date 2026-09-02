import { beforeEach, describe, expect, it, mock } from 'bun:test'
import { createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

let sessionUserId: string | undefined
let localRows: Array<{
  id: string
  lastMessagedAt: number
  lastUserMessage: string
}>
let cloudProps: { userId: string; localIds: ReadonlySet<string> } | null = null

mock.module('@/lib/auth/sessionStorage', () => ({
  useSessionInfo: () => ({
    sessionInfo: { user: sessionUserId ? { id: sessionUserId } : undefined },
  }),
}))
mock.module('@/modules/conversations/conversations-migration', () => ({
  useLegacyConversationMigration: () => {},
}))
mock.module('@/modules/conversations/conversations.hooks', () => ({
  useServerConversations: () => ({ data: localRows }),
  useDeleteServerConversation: () => ({ mutate: () => {} }),
}))
mock.module('./local/LocalChatHistory', () => ({
  LocalChatHistory: () => createElement('div', { 'data-testid': 'local' }),
}))
mock.module('./cloud/CloudChatHistory', () => ({
  CloudChatHistory: (props: {
    userId: string
    localIds: ReadonlySet<string>
  }) => {
    cloudProps = props
    return createElement('div', { 'data-testid': 'cloud' })
  },
}))

const { ChatHistory } = (await import('./ChatHistory')) as { ChatHistory: FC }

beforeEach(() => {
  sessionUserId = undefined
  localRows = []
  cloudProps = null
})

function render() {
  return renderToStaticMarkup(createElement(ChatHistory))
}

describe('ChatHistory', () => {
  it('always shows the local list', () => {
    expect(render()).toContain('data-testid="local"')
  })

  // Signed out there is no account to read, so the cloud section is absent
  // rather than empty.
  it('omits the cloud section when signed out', () => {
    expect(render()).not.toContain('data-testid="cloud"')
  })

  // It used to be one or the other: a signed-in user saw only the cloud and
  // could not see what their own machine was storing.
  it('shows both lists when signed in', () => {
    sessionUserId = 'user-1'
    const html = render()
    expect(html).toContain('data-testid="local"')
    expect(html).toContain('data-testid="cloud"')
  })

  it('puts the local list first', () => {
    sessionUserId = 'user-1'
    const html = render()
    expect(html.indexOf('data-testid="local"')).toBeLessThan(
      html.indexOf('data-testid="cloud"'),
    )
  })

  // One id space across the stores, so a conversation synced before sync was
  // turned off would otherwise appear in both lists.
  it('passes the local ids to the cloud section so it can deduplicate', () => {
    sessionUserId = 'user-1'
    localRows = [
      { id: 'a', lastMessagedAt: 1, lastUserMessage: 'hi' },
      { id: 'b', lastMessagedAt: 2, lastUserMessage: 'there' },
    ]
    render()
    expect(cloudProps?.userId).toBe('user-1')
    expect([...(cloudProps?.localIds ?? [])].sort()).toEqual(['a', 'b'])
  })

  // Two lists in one scroll area. Each list owning its own worked only while
  // exactly one of them ever rendered.
  it('renders a single scroll container for both lists', () => {
    sessionUserId = 'user-1'
    const html = render()
    expect((html.match(/<main/g) ?? []).length).toBe(1)
    expect(html).toContain('overflow-y-auto')
  })
})
