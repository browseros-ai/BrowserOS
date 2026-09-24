import { LLMConfigSchema } from '@browseros/shared/schemas/llm'
import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import { refinePrompt } from '../../lib/clients/llm/refine-prompt'
import type { ProviderRow } from '../../lib/db/schema'
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
 * whole configuration, including its API key.
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
  'getWithCredentials' | 'getDefaultWithCredentials' | 'listLlm'
>

interface RefinePromptRouteDeps {
  browserosId?: string
  /** Injectable so the resolution is testable without a database. */
  providerStore?: RefineProviderLookup
}

const NO_LLM_MESSAGE =
  'Connect an LLM provider in AI settings to refine a prompt'

type RefineProviderResolution =
  | { ok: true; row: ProviderRow | null }
  | { ok: false; message: string }

/**
 * Which provider refines the prompt.
 *
 * Refining needs an LLM, so the order is about not overriding a choice someone
 * already made, and about not failing when the selected target simply cannot
 * do this:
 *
 * 1. A named id is an explicit choice. An unknown one, or one naming a coding
 *    agent, is an error rather than a silent fall back to something else.
 * 2. A whole inline configuration with no id is an older extension that
 *    resolved the provider itself. That is its user's choice just as surely,
 *    so the selected provider is not read over the top of it.
 * 3. Otherwise the selection, and when the selection names a coding agent, the
 *    first LLM provider. The extension used to reach that same answer by
 *    resolving through a list that only ever held LLM providers, so a coding
 *    agent as the default has never stopped a prompt being refined.
 */
async function resolveRefineProvider(
  store: RefineProviderLookup,
  inline: { providerId?: string; provider?: string; model?: string },
): Promise<RefineProviderResolution> {
  if (inline.providerId) {
    const named = await store.getWithCredentials(inline.providerId)
    if (!named) {
      return { ok: false, message: `Unknown provider ${inline.providerId}` }
    }
    if (named.kind !== 'llm') {
      return {
        ok: false,
        message: `Provider ${named.id} is a coding agent and cannot refine a prompt`,
      }
    }
    return { ok: true, row: named }
  }

  if (inline.provider && inline.model) return { ok: true, row: null }

  const selected = await store.getDefaultWithCredentials()
  if (selected?.kind === 'llm') return { ok: true, row: selected }

  const [firstLlm] = await store.listLlm()
  if (!firstLlm) return { ok: false, message: NO_LLM_MESSAGE }
  const row = await store.getWithCredentials(firstLlm.id)
  return row ? { ok: true, row } : { ok: false, message: NO_LLM_MESSAGE }
}

export function createRefinePromptRoutes(deps: RefinePromptRouteDeps = {}) {
  const store = deps.providerStore ?? dbProviderStore

  return new Hono().post(
    '/',
    zValidator('json', RefinePromptRequestSchema),
    async (c) => {
      const { prompt, name, ...inline } = c.req.valid('json')

      const resolution = await resolveRefineProvider(store, inline)
      if (!resolution.ok) {
        return c.json({ success: false, message: resolution.message }, 400)
      }

      const llmConfig = resolution.row
        ? { ...inline, ...providerRowToLlmConfig(resolution.row) }
        : inline

      if (!llmConfig.provider || !llmConfig.model) {
        return c.json({ success: false, message: NO_LLM_MESSAGE }, 400)
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
