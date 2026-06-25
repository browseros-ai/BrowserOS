import { graphql } from '@/generated/graphql/gql'

export const GetRemoteLlmProvidersDocument = graphql(`
  query GetRemoteLlmProviders($profileId: String!) {
    llmProviders(condition: { profileId: $profileId }) {
      nodes {
        rowId
        type
        name
        baseUrl
        modelId
        supportsImages
        contextWindow
        temperature
        resourceName
        region
        createdAt
        updatedAt
      }
    }
  }
`)

export const DeleteRemoteLlmProviderDocument = graphql(`
  mutation DeleteRemoteLlmProvider($rowId: String!) {
    deleteLlmProvider(input: { rowId: $rowId }) {
      deletedLlmProviderId
    }
  }
`)
