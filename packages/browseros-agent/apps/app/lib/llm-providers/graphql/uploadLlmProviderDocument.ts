import { graphql } from '@/generated/graphql/gql'

export const CreateLlmProviderForUploadDocument = graphql(`
  mutation CreateLlmProviderForUpload($input: CreateLlmProviderInput!) {
    createLlmProvider(input: $input) {
      llmProvider {
        rowId
      }
    }
  }
`)

export const UpdateLlmProviderForUploadDocument = graphql(`
  mutation UpdateLlmProviderForUpload($input: UpdateLlmProviderInput!) {
    updateLlmProvider(input: $input) {
      llmProvider {
        rowId
      }
    }
  }
`)

export const GetLlmProvidersByProfileIdDocument = graphql(`
  query GetLlmProvidersByProfileId($profileId: String!) {
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
      }
    }
  }
`)
