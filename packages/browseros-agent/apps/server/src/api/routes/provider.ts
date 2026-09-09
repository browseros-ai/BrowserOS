/** Tests a stored provider by ID, or resolves an unsaved draft without writing it. */
import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import { testProviderConnection } from '../../lib/clients/llm/test-provider'
import { logger } from '../../lib/logger'
import {
  ProviderConfigError,
  providerToLlmConfig,
  resolveProviderConfig,
} from '../../lib/providers/provider-config'
import {
  dbProviderStore,
  type ProviderStore,
} from '../../lib/providers/provider-store'
import { AgentLLMConfigSchema } from '../types'

interface ProviderRouteDeps {
  browserosId?: string
  store?: Pick<ProviderStore, 'getWithCredentials'>
  testConnection?: typeof testProviderConnection
}

// Keep the full-config shape for draft tests and independently updating older
// extensions. An ID-only request always tests the authoritative saved row.
const TestRequestSchema = z.union([
  z.object({ providerId: z.string().min(1) }).strict(),
  // Incomplete drafts reach the shared resolver for actionable field errors.
  AgentLLMConfigSchema.extend({ model: z.string().optional() }),
])

export function createProviderRoutes(deps: ProviderRouteDeps = {}): Hono {
  const store = deps.store ?? dbProviderStore
  const testConnection = deps.testConnection ?? testProviderConnection
  return new Hono().post(
    '/',
    zValidator('json', TestRequestSchema, (result, c) => {
      if (!result.success)
        return c.json(
          {
            success: false,
            message: 'Provide a saved provider ID or valid provider settings.',
          },
          400,
        )
      return undefined
    }),
    async (c) => {
      const request = c.req.valid('json')
      const stored = request.providerId
        ? await store.getWithCredentials(request.providerId)
        : null
      if (stored && stored.kind !== 'llm')
        return c.json(
          { success: false, message: 'This provider is a coding agent.' },
          400,
        )
      try {
        const changes =
          'provider' in request
            ? {
                id: request.providerId ?? 'draft',
                name: stored?.name ?? 'Test',
                type: request.provider,
                modelId: request.model,
                contextWindow: stored?.contextWindow ?? 128000,
                baseUrl: request.baseUrl,
                headers: request.headers,
                apiKey: request.apiKey,
                resourceName: request.resourceName,
                region: request.region,
                accessKeyId: request.accessKeyId,
                secretAccessKey: request.secretAccessKey,
                sessionToken: request.sessionToken,
                reasoningEffort: request.reasoningEffort,
                reasoningSummary: request.reasoningSummary,
              }
            : stored
        if (!changes) {
          return c.json(
            {
              success: false,
              message:
                'This provider no longer exists. Reload the provider list.',
            },
            404,
          )
        }
        const resolved = providerToLlmConfig(
          resolveProviderConfig(changes, stored),
        )
        const result = await testConnection(resolved, deps.browserosId)
        logger.info('Provider test result', {
          provider: resolved.provider,
          model: resolved.model,
          success: result.success,
          responseTime: result.responseTime,
        })
        return c.json(result, result.success ? 200 : 400)
      } catch (error) {
        if (!(error instanceof ProviderConfigError)) throw error
        return c.json(
          {
            success: false,
            message: error.message,
            fieldErrors: error.fieldErrors,
          },
          400,
        )
      }
    },
  )
}
