import { execute } from '@/lib/graphql/execute'
import { sessionStorage } from '../auth/sessionStorage'
import { sentry } from '../sentry/sentry'
import type { Conversation } from './conversationStorage'
import {
  BulkCreateConversationMessagesDocument,
  ConversationExistsDocument,
  CreateConversationForUploadDocument,
  GetProfileIdByUserIdDocument,
  GetUploadedMessageCountDocument,
} from './graphql/uploadConversationDocument'

/**
 * Uploads each conversation to the cloud idempotently (creating it and
 * appending only the messages the cloud does not already have) and returns the
 * ids that are now fully in the cloud. A missing session/profile yields an empty
 * result and per-conversation errors are swallowed and omitted, so a caller can
 * safely retry whatever is not returned. Does not touch local storage.
 *
 * Pass `expectedUserId` to bind the upload to a specific account: if the live
 * session no longer belongs to that user (a session switch races the upload),
 * nothing is uploaded, so one account's buffered conversation is never created
 * under another account's profile.
 */
export async function uploadConversations(
  conversations: Conversation[],
  expectedUserId?: string,
): Promise<string[]> {
  if (conversations.length === 0) return []

  const sessionInfo = await sessionStorage.getValue()
  const userId = sessionInfo?.user?.id
  if (!userId) return []
  if (expectedUserId && userId !== expectedUserId) return []

  const profileResult = await execute(GetProfileIdByUserIdDocument, { userId })
  const profileId = profileResult.profileByUserId?.rowId
  if (!profileId) return []

  const uploadedIds: string[] = []

  for (const conversation of conversations) {
    try {
      const existsResult = await execute(ConversationExistsDocument, {
        pConversationId: conversation.id,
      })

      let uploadedCount = 0

      if (existsResult.conversationExists) {
        const countResult = await execute(GetUploadedMessageCountDocument, {
          conversationId: conversation.id,
        })
        uploadedCount = countResult.conversationMessages?.totalCount ?? 0

        if (uploadedCount >= conversation.messages.length) {
          uploadedIds.push(conversation.id)
          continue
        }
      } else {
        await execute(CreateConversationForUploadDocument, {
          input: {
            conversation: {
              rowId: conversation.id,
              profileId,
              lastMessagedAt: new Date(
                conversation.lastMessagedAt,
              ).toISOString(),
              createdAt: new Date(conversation.lastMessagedAt).toISOString(),
            },
          },
        })
      }

      const remainingMessages = conversation.messages.slice(uploadedCount)

      if (remainingMessages.length > 0) {
        const BATCH_SIZE = 50
        for (let i = 0; i < remainingMessages.length; i += BATCH_SIZE) {
          const batch = remainingMessages.slice(i, i + BATCH_SIZE)
          await execute(BulkCreateConversationMessagesDocument, {
            input: {
              pConversationId: conversation.id,
              pMessages: batch.map((msg, batchIndex) => ({
                orderIndex: uploadedCount + i + batchIndex,
                message: msg,
              })),
            },
          })
        }
      }

      uploadedIds.push(conversation.id)
    } catch (error) {
      sentry.captureException(error, {
        extra: {
          conversationId: conversation.id,
          messageCount: conversation.messages.length,
        },
      })
    }
  }

  return uploadedIds
}
