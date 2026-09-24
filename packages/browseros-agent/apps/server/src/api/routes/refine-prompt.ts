import { LLMConfigSchema } from '@browseros/shared/schemas/llm'
import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import { refinePrompt } from '../../lib/clients/llm/refine-prompt'
import { logger } from '../../lib/logger'
import {
  dbProviderStore,
  type ProviderStore,
} from '../../lib/providers/provider-store'
import { providerRowToLlmConfig } from '../services/chat-provider-config'

/**
 * The provider is named, not described, the way /chat names one.
 *
 * Every inline field stays optional and accepted: the extension updates
 * independently of the browser binary, so a shipped build may still send the
 * whole configuration, including its API key. The stored row wins where both
 * are present, because it is the source of truth.
 */
const RefinePromptRequestSchema = LLMConfigSchema.partial().extend({
  prompt: z.string().min(1, 'Prompt cannot be empty'),
  name: z.string().min(1, 'Task name cannot be empty'),
  model: z.string().min(1).optional(),
  upstreamProvider: z.string().optional(),
  providerId: z.string().optional(),
})

type RefineProviderLookup = Pick<
  ProviderStore,
  'getWithCredentials' | 'getDefaultWithCredentials'
>

interface RefinePromptRouteDeps {
  browserosId?: string
  /** Injectable so the resolution is testable without a database. */
  providerStore?: RefineProviderLookup
}

export function createRefinePromptRoutes(deps: RefinePromptRouteDeps = {}) {
  const store = deps.providerStore ?? dbProviderStore

  return new Hono().post(
    '/',
    zValidator('json', RefinePromptRequestSchema),
    async (c) => {
      const { prompt, name, ...inline } = c.req.valid('json')

      const row = inline.providerId
        ? await store.getWithCredentials(inline.providerId)
        : await store.getDefaultWithCredentials()

      if (row && row.kind !== 'llm') {
        return c.json(
          {
            success: false,
            message: `Provider ${row.id} is a coding agent and cannot refine a prompt`,
          },
          400,
        )
      }

      const llmConfig = row
        ? { ...inline, ...providerRowToLlmConfig(row) }
        : inline

      if (!llmConfig.provider || !llmConfig.model) {
        return c.json(
          {
            success: false,
            message: inline.providerId
              ? `Unknown provider ${inline.providerId}`
              : 'No provider given and none is selected',
          },
          400,
        )
      }

      logger.info('Refine prompt request', {
        provider: llmConfig.provider,
        model: llmConfig.model,
        taskName: name,
      })

      const result = await refinePrompt(
        { ...llmConfig, provider: llmConfig.provider, model: llmConfig.model },
        { prompt, name },
        deps.browserosId,
      )

      logger.info('Refine prompt result', {
        provider: llmConfig.provider,
        success: result.success,
      })

      return c.json(result, result.success ? 200 : 400)
    },
  )
}
