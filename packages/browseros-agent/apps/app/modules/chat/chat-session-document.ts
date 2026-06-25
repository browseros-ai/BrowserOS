import { graphql } from '@/generated/graphql/gql'

export const CreateConversationWithMessageDocument = graphql(`
  mutation CreateConversationWithMessage(
    $conversationId: String!
    $profileId: String!
    $message: JSON!
  ) {
    createConversation(
      input: {
        conversation: {
          rowId: $conversationId
          profileId: $profileId
          lastMessagedAt: "now()"
        }
      }
    ) {
      conversation {
        rowId
      }
    }
    createConversationMessage(
      input: {
        conversationMessage: {
          rowId: $conversationId
          conversationId: $conversationId
          orderIndex: 0
          message: $message
        }
      }
    ) {
      conversationMessage {
        rowId
        orderIndex
      }
    }
  }
`)

export const AppendConversationMessageDocument = graphql(`
  mutation AppendConversationMessage(
    $messageId: String!
    $conversationId: String!
    $orderIndex: Int!
    $message: JSON!
  ) {
    createConversationMessage(
      input: {
        conversationMessage: {
          rowId: $messageId
          conversationId: $conversationId
          orderIndex: $orderIndex
          message: $message
        }
      }
    ) {
      conversationMessage {
        rowId
        orderIndex
      }
    }
  }
`)

export const UpdateConversationLastMessagedAtDocument = graphql(`
  mutation UpdateConversationLastMessagedAt($conversationId: String!) {
    updateConversation(
      input: { rowId: $conversationId, patch: { lastMessagedAt: "now()" } }
    ) {
      conversation {
        rowId
        lastMessagedAt
      }
    }
  }
`)

export const GetConversationWithMessagesDocument = graphql(`
  query GetConversationWithMessages($conversationId: String!) {
    conversation(rowId: $conversationId) {
      rowId
      conversationMessages(first: 100, orderBy: ORDER_INDEX_ASC) {
        nodes {
          message
        }
      }
    }
  }
`)
