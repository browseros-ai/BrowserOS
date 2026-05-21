import { storage } from '@wxt-dev/storage'
import type { UIMessage } from 'ai'
import { useEffect, useState } from 'react'
import { decryptObject, encryptObject } from '../crypto'
import { useSessionInfo } from '../auth/sessionStorage'
import { removeConversationExecutionHistory } from '../execution-history/storage'
import { uploadConversationsToGraphql } from './uploadConversationsToGraphql'

const MAX_CONVERSATIONS = 50

export interface Conversation {
  id: string
  messages: UIMessage[]
  lastMessagedAt: number
}

const rawConversationStorage = storage.defineItem<string>(
  'local:conversations-encrypted',
  {
    fallback: '',
  },
)

export const conversationStorage = {
  ...rawConversationStorage,
  getValue: async () => {
    const encrypted = await rawConversationStorage.getValue()
    if (!encrypted) return []
    return (await decryptObject<Conversation[]>(encrypted)) ?? []
  },
  setValue: async (conversations: Conversation[]) => {
    const encrypted = await encryptObject(conversations)
    return rawConversationStorage.setValue(encrypted)
  },
  watch: (callback: (newValue: Conversation[] | null) => void) => {
    return rawConversationStorage.watch(async (newValue) => {
      if (!newValue) {
        callback([])
        return
      }
      const decrypted = await decryptObject<Conversation[]>(newValue)
      callback(decrypted ?? [])
    })
  }
}


export function useConversations() {
  const [conversations, setConversations] = useState<Conversation[]>([])

  const { sessionInfo } = useSessionInfo()

  useEffect(() => {
    if (sessionInfo.user?.id && conversations.length > 0) {
      uploadConversationsToGraphql(conversations)
    }
  }, [sessionInfo.user?.id, conversations])

  useEffect(() => {
    conversationStorage.getValue().then(setConversations)
    const unwatch = conversationStorage.watch((newValue) => {
      setConversations(newValue ?? [])
    })
    return unwatch
  }, [])

  const removeConversation = async (id: string) => {
    const current = (await conversationStorage.getValue()) ?? []
    await conversationStorage.setValue(current.filter((c) => c.id !== id))
    await removeConversationExecutionHistory(id)
  }

  const saveConversation = async (id: string, messages: UIMessage[]) => {
    const current = (await conversationStorage.getValue()) ?? []
    const existingIndex = current.findIndex((c) => c.id === id)

    if (existingIndex >= 0) {
      const existing = current[existingIndex]
      const hasContentChanged =
        existing.messages.length !== messages.length ||
        JSON.stringify(existing.messages) !== JSON.stringify(messages)

      if (!hasContentChanged) return

      current[existingIndex] = {
        ...existing,
        messages,
        lastMessagedAt: Date.now(),
      }
      await conversationStorage.setValue(current)
    } else {
      const newConversation: Conversation = {
        id,
        messages,
        lastMessagedAt: Date.now(),
      }
      const nextConversations = [newConversation, ...current]
      const removedConversations = nextConversations.slice(MAX_CONVERSATIONS)
      await conversationStorage.setValue(
        nextConversations.slice(0, MAX_CONVERSATIONS),
      )
      await Promise.all(
        removedConversations.map((conversation) =>
          removeConversationExecutionHistory(conversation.id),
        ),
      )
    }
  }

  const getConversation = (id: string) => {
    return conversations.find((c) => c.id === id)
  }

  return {
    conversations,
    removeConversation,
    saveConversation,
    getConversation,
  }
}
