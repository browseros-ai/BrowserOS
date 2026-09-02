import type { FC } from 'react'
import { useMemo } from 'react'
import { useSessionInfo } from '@/lib/auth/sessionStorage'
import { useServerConversations } from '@/modules/conversations/conversations.hooks'
import { CloudChatHistory } from './cloud/CloudChatHistory'
import { LocalChatHistory } from './local/LocalChatHistory'

/**
 * History is the union of what is on this machine and what is still in the
 * account, with the local list first and always present.
 *
 * It used to be one or the other: signed in showed only the cloud, signed out
 * showed only the local server. That meant a signed-in user could not see the
 * conversations their own machine was storing.
 */
export const ChatHistory: FC = () => {
  const { sessionInfo } = useSessionInfo()
  const userId = sessionInfo.user?.id
  // Same query key as LocalChatHistory, so this shares its cache rather than
  // fetching a second time. Only the ids are needed, to keep a conversation
  // that exists in both places from being listed twice.
  const { data: localConversations = [] } = useServerConversations()
  const localIds = useMemo(
    () => new Set(localConversations.map((conversation) => conversation.id)),
    [localConversations],
  )

  // One scroll area for both lists. Each list used to own its own, which
  // worked while only ever one of them rendered.
  return (
    <main className="mt-4 flex h-full flex-1 flex-col overflow-y-auto">
      <LocalChatHistory />
      {userId ? <CloudChatHistory userId={userId} localIds={localIds} /> : null}
    </main>
  )
}
