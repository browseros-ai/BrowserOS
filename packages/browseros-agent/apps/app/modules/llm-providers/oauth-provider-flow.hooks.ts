import { useEffect, useRef } from 'react'
import { toast } from 'sonner'
import { CHATGPT_PROVIDER_DISPLAY_NAME } from '@/lib/llm-providers/provider-display-names'
import { getProviderTemplate } from '@/lib/llm-providers/providerTemplates'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { track } from '@/lib/metrics/track'
import { useOAuthStatus } from '@/modules/llm-providers/oauth-status.hooks'

export interface OAuthProviderFlowConfig {
  providerType: 'chatgpt-pro'
  displayName: string
  startedEvent: string
  completedEvent: string
  disconnectedEvent: string
}

export interface OAuthProviderFlowReturn {
  disconnect: () => Promise<void>
  startOAuthFlow: (agentServerUrl: string | undefined) => Promise<void>
}

export interface SaveOAuthProviderInput {
  config: OAuthProviderFlowConfig
  saveProvider: (provider: LlmProviderConfig) => Promise<void> | void
  now?: number
}

/** Persists the local provider row created after an OAuth account authenticates. */
export async function saveOAuthProviderFromStatus({
  config,
  saveProvider,
  now = Date.now(),
}: SaveOAuthProviderInput): Promise<LlmProviderConfig> {
  const template = getProviderTemplate(config.providerType)
  const provider: LlmProviderConfig = {
    id: `${config.providerType}-${now}`,
    type: config.providerType,
    name: CHATGPT_PROVIDER_DISPLAY_NAME,
    modelId: template?.defaultModelId ?? '',
    supportsImages: template?.supportsImages ?? true,
    contextWindow: template?.contextWindow ?? 128000,
    temperature: 0.2,
    createdAt: now,
    updatedAt: now,
    reasoningEffort: 'medium',
    reasoningSummary: 'auto',
  }

  await saveProvider(provider)
  return provider
}

/** Coordinates OAuth launch, status polling, and local provider creation. */
export function useOAuthProviderFlow(
  config: OAuthProviderFlowConfig,
  saveProvider: (provider: LlmProviderConfig) => Promise<void> | void,
): OAuthProviderFlowReturn {
  const { status, startPolling, disconnect } = useOAuthStatus(
    config.providerType,
  )
  const flowStartedRef = useRef(false)
  const providerSaveRef = useRef<Promise<LlmProviderConfig> | null>(null)

  useEffect(() => {
    if (!status?.authenticated) return
    if (!flowStartedRef.current) return
    if (providerSaveRef.current) return

    providerSaveRef.current = saveOAuthProviderFromStatus({
      config,
      saveProvider,
    })

    providerSaveRef.current
      .then(() => {
        track(config.completedEvent, { email: status.email })
        toast.success(`${config.displayName} Connected`, {
          description: status.email
            ? `Authenticated as ${status.email}`
            : `Successfully authenticated with ${config.displayName}`,
        })
        flowStartedRef.current = false
      })
      .catch((err) => {
        toast.error(`Failed to create ${config.displayName} provider`, {
          description: err instanceof Error ? err.message : 'Unknown error',
        })
      })
      .finally(() => {
        providerSaveRef.current = null
      })
  }, [config, saveProvider, status])

  async function startOAuthFlow(agentServerUrl: string | undefined) {
    if (!agentServerUrl) {
      toast.error('Server not available', {
        description: 'Cannot start OAuth flow without server connection.',
      })
      return
    }

    flowStartedRef.current = true

    try {
      await handleServerAuth(agentServerUrl)
    } catch (err) {
      flowStartedRef.current = false
      toast.error(`Failed to start ${config.displayName} authentication`, {
        description: err instanceof Error ? err.message : 'Unknown error',
      })
    }
  }

  async function handleServerAuth(agentServerUrl: string) {
    // The response is a redirect to the sign-in page. Fetch follows it so we
    // can open the final URL, which isn't a JSON operation in the client.
    const res = await fetch(
      `${agentServerUrl}/oauth/${config.providerType}/start`,
    )

    if (
      !res.ok &&
      res.headers.get('content-type')?.includes('application/json')
    ) {
      const body: unknown = await res.json()
      if (
        typeof body === 'object' &&
        body !== null &&
        'error' in body &&
        typeof body.error === 'string'
      )
        throw new Error(body.error)
    }

    if (!res.ok) throw new Error(`Server returned ${res.status}`)
    window.open(res.url, '_blank')
    startPolling()
    track(config.startedEvent)
    toast.info(`Authenticating with ${config.displayName}`, {
      description: 'Complete the login in the opened tab.',
    })
  }

  return {
    disconnect,
    startOAuthFlow,
  }
}
