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

export async function uploadConversationsToGraphql(
  conversations: Conversation[],
) {
  if (conversations.length === 0) return

  const sessionInfo = await sessionStorage.getValue()
  const userId = sessionInfo?.user?.id
  if (!userId) return

  const profileResult = await execute(GetProfileIdByUserIdDocument, { userId })
  const profileId = profileResult.profileByUserId?.rowId
  if (!profileId) return

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
    } catch (error) {
      sentry.captureException(error, {
        extra: {
          conversationId: conversation.id,
          messageCount: conversation.messages.length,
        },
      })
    }
  }
  // Local conversations are the durable source of truth and are intentionally
  // NOT drained after upload (issue #559): the cloud is a mirror, not a move.
  // Re-uploads are idempotent via the conversationExists + uploaded-count
  // checks above, so retaining local costs at most a skip round-trip.
}
