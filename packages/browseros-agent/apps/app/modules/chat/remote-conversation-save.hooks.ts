import type { UIMessage } from 'ai'
import { useCallback, useRef } from 'react'
import { useSessionInfo } from '@/lib/auth/sessionStorage'
import { GetProfileIdByUserIdDocument } from '@/lib/conversations/graphql/uploadConversationDocument'
import { execute } from '@/lib/graphql/execute'
import { sentry } from '@/lib/sentry/sentry'
import {
  AppendConversationMessageDocument,
  CreateConversationWithMessageDocument,
  UpdateConversationLastMessagedAtDocument,
} from './chat-session-document'

export function useRemoteConversationSave() {
  const { sessionInfo } = useSessionInfo()
  const userId = sessionInfo.user?.id

  const profileIdRef = useRef<string | null>(null)
  const createdConversationsRef = useRef<Set<string>>(new Set())
  const savedMessageIdsRef = useRef<Set<string>>(new Set())

  const getProfileId = async (): Promise<string | null> => {
    if (profileIdRef.current) return profileIdRef.current
    if (!userId) return null

    const result = await execute(GetProfileIdByUserIdDocument, { userId })
    const profileId = result.profileByUserId?.rowId ?? null
    profileIdRef.current = profileId
    return profileId
  }

  const saveConversation = async (
    conversationId: string,
    messages: UIMessage[],
  ) => {
    if (!userId || messages.length === 0) return

    const profileId = await getProfileId()
    if (!profileId) return

    const isNewConversation =
      !createdConversationsRef.current.has(conversationId)
    const newMessages = messages.filter(
      (msg) => !savedMessageIdsRef.current.has(msg.id),
    )

    if (newMessages.length === 0) return

    try {
      if (isNewConversation && newMessages.length > 0) {
        const firstMessage = newMessages[0]
        await execute(CreateConversationWithMessageDocument, {
          conversationId,
          profileId,
          message: firstMessage,
        })
        createdConversationsRef.current.add(conversationId)
        savedMessageIdsRef.current.add(firstMessage.id)

        for (let i = 1; i < newMessages.length; i++) {
          const msg = newMessages[i]
          const orderIndex = messages.findIndex((m) => m.id === msg.id)
          await execute(AppendConversationMessageDocument, {
            messageId: msg.id,
            conversationId,
            orderIndex,
            message: msg,
          })
          savedMessageIdsRef.current.add(msg.id)
        }
      } else {
        for (const msg of newMessages) {
          const orderIndex = messages.findIndex((m) => m.id === msg.id)
          await execute(AppendConversationMessageDocument, {
            messageId: msg.id,
            conversationId,
            orderIndex,
            message: msg,
          })
          savedMessageIdsRef.current.add(msg.id)
        }

        await execute(UpdateConversationLastMessagedAtDocument, {
          conversationId,
        })
      }
    } catch (error) {
      sentry.captureException(error, {
        extra: {
          message: 'Failed to save conversation to remote',
        },
      })
    }
  }

  const resetConversation = () => {
    savedMessageIdsRef.current = new Set()
  }

  const markMessagesAsSaved = useCallback(
    (conversationId: string, messages: UIMessage[]) => {
      createdConversationsRef.current.add(conversationId)
      for (const msg of messages) {
        savedMessageIdsRef.current.add(msg.id)
      }
    },
    [],
  )

  return {
    isLoggedIn: !!userId,
    saveConversation,
    resetConversation,
    markMessagesAsSaved,
  }
}
