import { useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import {
  type ClientAuthConfig,
  requestDeviceCode,
  startTokenPolling,
} from '@/lib/llm-providers/client-oauth'
import { CHATGPT_PROVIDER_DISPLAY_NAME } from '@/lib/llm-providers/provider-display-names'
import { getProviderTemplate } from '@/lib/llm-providers/providerTemplates'
import type { LlmProviderConfig, ProviderType } from '@/lib/llm-providers/types'
import { track } from '@/lib/metrics/track'
import { useOAuthStatus } from '@/modules/llm-providers/oauth-status.hooks'

export interface OAuthProviderFlowConfig {
  providerType: ProviderType
  displayName: string
  startedEvent: string
  completedEvent: string
  disconnectedEvent: string
  /** Client-side auth for providers with WAF-protected endpoints */
  clientAuth?: ClientAuthConfig
}

export interface PendingDeviceCode {
  userCode: string
  providerName: string
  verificationUri: string
}

export interface OAuthProviderFlowReturn {
  status: { authenticated: boolean; email?: string } | null
  disconnect: () => Promise<void>
  startOAuthFlow: (agentServerUrl: string | undefined) => Promise<void>
  pendingDeviceCode: PendingDeviceCode | null
  clearDeviceCode: () => void
}

export interface SaveOAuthProviderInput {
  config: OAuthProviderFlowConfig
  status: { email?: string }
  saveProvider: (provider: LlmProviderConfig) => Promise<void> | void
  now?: number
}

/** Persists the local provider row created after an OAuth account authenticates. */
export async function saveOAuthProviderFromStatus({
  config,
  status,
  saveProvider,
  now = Date.now(),
}: SaveOAuthProviderInput): Promise<LlmProviderConfig> {
  const template = getProviderTemplate(config.providerType)
  const providerName =
    config.providerType === 'chatgpt-pro'
      ? CHATGPT_PROVIDER_DISPLAY_NAME
      : `${config.displayName}${status.email ? ` (${status.email})` : ''}`
  const provider: LlmProviderConfig = {
    id: `${config.providerType}-${now}`,
    type: config.providerType,
    name: providerName,
    modelId: template?.defaultModelId ?? '',
    supportsImages: template?.supportsImages ?? true,
    contextWindow: template?.contextWindow ?? 128000,
    temperature: 0.2,
    createdAt: now,
    updatedAt: now,
  }
  if (config.providerType === 'chatgpt-pro') {
    provider.reasoningEffort = 'medium'
    provider.reasoningSummary = 'auto'
  }

  await saveProvider(provider)
  return provider
}

/** Coordinates OAuth launch, status polling, and local provider creation. */
export function useOAuthProviderFlow(
  config: OAuthProviderFlowConfig,
  _providers: LlmProviderConfig[],
  saveProvider: (provider: LlmProviderConfig) => Promise<void> | void,
): OAuthProviderFlowReturn {
  const { status, startPolling, disconnect } = useOAuthStatus(
    config.providerType,
  )
  const flowStartedRef = useRef(false)
  const providerSaveRef = useRef<Promise<LlmProviderConfig> | null>(null)
  const [pendingDeviceCode, setPendingDeviceCode] =
    useState<PendingDeviceCode | null>(null)

  useEffect(() => {
    if (!status?.authenticated) return
    if (!flowStartedRef.current) return
    if (providerSaveRef.current) return

    providerSaveRef.current = saveOAuthProviderFromStatus({
      config,
      status,
      saveProvider,
    })

    providerSaveRef.current
      .then(() => {
        setPendingDeviceCode(null)
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
      if (config.clientAuth) {
        await handleClientAuth(config.clientAuth, agentServerUrl)
      } else {
        await handleServerAuth(agentServerUrl)
      }
    } catch (err) {
      flowStartedRef.current = false
      toast.error(`Failed to start ${config.displayName} authentication`, {
        description: err instanceof Error ? err.message : 'Unknown error',
      })
    }
  }

  async function handleClientAuth(auth: ClientAuthConfig, serverUrl: string) {
    const { deviceData, codeVerifier } = await requestDeviceCode(auth)

    const verificationUri =
      deviceData.verification_uri_complete ?? deviceData.verification_uri
    window.open(verificationUri, '_blank')
    track(config.startedEvent)
    setPendingDeviceCode({
      userCode: deviceData.user_code,
      providerName: config.displayName,
      verificationUri,
    })

    startTokenPolling(auth, deviceData, codeVerifier, async (token) => {
      await fetch(`${serverUrl}/oauth/${config.providerType}/token`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(token),
      })
      startPolling()
    })
  }

  async function handleServerAuth(agentServerUrl: string) {
    const res = await fetch(
      `${agentServerUrl}/oauth/${config.providerType}/start`,
    )

    if (res.headers.get('content-type')?.includes('application/json')) {
      const data = (await res.json()) as {
        userCode?: string
        verificationUri?: string
        error?: string
      }

      if (!res.ok || data.error) {
        throw new Error(data.error || `Server returned ${res.status}`)
      }
      if (!data.userCode || !data.verificationUri) {
        throw new Error('Invalid response from server')
      }

      window.open(data.verificationUri, '_blank')
      startPolling()
      track(config.startedEvent)
      setPendingDeviceCode({
        userCode: data.userCode,
        providerName: config.displayName,
        verificationUri: data.verificationUri,
      })
      return
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
    status,
    disconnect,
    startOAuthFlow,
    pendingDeviceCode,
    clearDeviceCode: () => setPendingDeviceCode(null),
  }
}
