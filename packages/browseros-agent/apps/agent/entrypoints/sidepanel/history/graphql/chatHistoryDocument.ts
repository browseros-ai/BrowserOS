import { graphql } from '@/generated/graphql/gql'

export const GetConversationsForHistoryDocument = graphql(`
  query GetConversationsForHistory($profileId: String!, $first: Int = 50, $after: Cursor) {
    conversations(
      condition: { profileId: $profileId }
      first: $first
      after: $after
      orderBy: LAST_MESSAGED_AT_DESC
    ) {
      nodes {
        rowId
        lastMessagedAt
        conversationMessages(first: 2, orderBy: ORDER_INDEX_DESC) {
          nodes {
            message
          }
        }
      }
      pageInfo {
        endCursor
        hasNextPage
      }
    }
  }
`)

export const DeleteConversationDocument = graphql(`
  mutation DeleteConversation($rowId: String!) {
    deleteConversation(input: { rowId: $rowId }) {
      deletedConversationId
    }
  }
`)
