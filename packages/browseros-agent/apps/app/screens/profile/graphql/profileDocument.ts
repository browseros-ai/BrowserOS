import { graphql } from '@/generated/graphql/gql'

export const GetProfileByUserIdDocument = graphql(`
  query GetProfileByUserId($userId: String!) {
    profileByUserId(userId: $userId) {
      rowId
      firstName
      lastName
      avatarUrl
    }
  }
`)

export const UpdateProfileByUserIdDocument = graphql(`
  mutation UpdateProfileByUserId($userId: String!, $patch: ProfilePatch!) {
    updateProfileByUserId(input: { userId: $userId, patch: $patch }) {
      profile {
        rowId
        firstName
        lastName
        avatarUrl
      }
    }
  }
`)
