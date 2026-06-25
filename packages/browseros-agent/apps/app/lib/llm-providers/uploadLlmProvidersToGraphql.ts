import { isEqual, omit } from 'es-toolkit'
import { GetProfileIdByUserIdDocument } from '@/lib/conversations/graphql/uploadConversationDocument'
import { execute } from '@/lib/graphql/execute'
import { sentry } from '@/lib/sentry/sentry'
import {
  CreateLlmProviderForUploadDocument,
  GetLlmProvidersByProfileIdDocument,
  UpdateLlmProviderForUploadDocument,
} from './graphql/uploadLlmProviderDocument'
import type { LlmProviderConfig } from './types'

type RemoteProvider = {
  rowId: string
  type: string
  name: string
  baseUrl: string | null
  modelId: string
  supportsImages: boolean
  contextWindow: number | null
  temperature: number | null
  resourceName: string | null
  region: string | null
}

const IGNORED_FIELDS = [
  'id',
  'createdAt',
  'updatedAt',
  'apiKey',
  'accessKeyId',
  'secretAccessKey',
  'sessionToken',
] as const

function toComparable(provider: LlmProviderConfig) {
  const data = omit(provider, IGNORED_FIELDS)
  return {
    ...data,
    baseUrl: data.baseUrl ?? null,
    resourceName: data.resourceName ?? null,
    region: data.region ?? null,
  }
}

export async function uploadLlmProvidersToGraphql(
  providers: LlmProviderConfig[],
  userId: string,
) {
  if (providers.length === 0) return

  const profileResult = await execute(GetProfileIdByUserIdDocument, { userId })
  const profileId = profileResult.profileByUserId?.rowId
  if (!profileId) return

  const remoteResult = await execute(GetLlmProvidersByProfileIdDocument, {
    profileId,
  })
  const remoteProviders = new Map<string, RemoteProvider>()
  for (const node of remoteResult.llmProviders?.nodes ?? []) {
    if (node) {
      remoteProviders.set(node.rowId, node as RemoteProvider)
    }
  }

  for (const provider of providers) {
    if (provider.type === 'browseros') continue

    try {
      const remote = remoteProviders.get(provider.id)

      if (remote) {
        if (isEqual(toComparable(provider), omit(remote, ['rowId']))) continue

        await execute(UpdateLlmProviderForUploadDocument, {
          input: {
            rowId: provider.id,
            patch: {
              type: provider.type,
              name: provider.name,
              baseUrl: provider.baseUrl ?? null,
              modelId: provider.modelId,
              supportsImages: provider.supportsImages,
              contextWindow: provider.contextWindow,
              temperature: provider.temperature,
              resourceName: provider.resourceName ?? null,
              region: provider.region ?? null,
              updatedAt: new Date(provider.updatedAt).toISOString(),
            },
          },
        })
      } else {
        await execute(CreateLlmProviderForUploadDocument, {
          input: {
            llmProvider: {
              rowId: provider.id,
              profileId,
              type: provider.type,
              name: provider.name,
              baseUrl: provider.baseUrl ?? null,
              modelId: provider.modelId,
              supportsImages: provider.supportsImages,
              contextWindow: provider.contextWindow,
              temperature: provider.temperature,
              resourceName: provider.resourceName ?? null,
              region: provider.region ?? null,
              createdAt: new Date(provider.createdAt).toISOString(),
              updatedAt: new Date(provider.updatedAt).toISOString(),
            },
          },
        })
      }
    } catch (error) {
      sentry.captureException(error, {
        extra: {
          providerId: provider.id,
          providerName: provider.name,
        },
      })
    }
  }
}
