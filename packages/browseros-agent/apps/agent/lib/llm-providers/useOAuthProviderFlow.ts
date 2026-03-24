import { useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import { track } from '@/lib/metrics/track'
import {
  type ClientAuthConfig,
  requestDeviceCode,
  startTokenPolling,
} from './client-oauth'
import { getProviderTemplate } from './providerTemplates'
import type { LlmProviderConfig, ProviderType } from './types'
import { useOAuthStatus } from './useOAuthStatus'

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

interface OAuthProviderFlowReturn {
  status: { authenticated: boolean; email?: string } | null
  disconnect: () => Promise<void>
  startOAuthFlow: (agentServerUrl: string | undefined) => Promise<void>
  pendingDeviceCode: PendingDeviceCode | null
  clearDeviceCode: () => void
}

export function useOAuthProviderFlow(
  config: OAuthProviderFlowConfig,
  providers: LlmProviderConfig[],
  saveProvider: (provider: LlmProviderConfig) => Promise<void> | void,
): OAuthProviderFlowReturn {
  const { status, startPolling, disconnect } = useOAuthStatus(
    config.providerType,
  )
  const flowStartedRef = useRef(false)
  const [pendingDeviceCode, setPendingDeviceCode] =
    useState<PendingDeviceCode | null>(null)

  // Auto-create provider when OAuth completes
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — only trigger on auth status change
  useEffect(() => {
    if (!status?.authenticated) return
    if (!flowStartedRef.current) return
    if (providers.some((p) => p.type === config.providerType)) return

    const now = Date.now()
    try {
      const template = getProviderTemplate(config.providerType)
      saveProvider({
        id: `${config.providerType}-${now}`,
        type: config.providerType,
        name: `${config.displayName}${status.email ? ` (${status.email})` : ''}`,
        modelId: template?.defaultModelId ?? '',
        supportsImages: template?.supportsImages ?? true,
        contextWindow: template?.contextWindow ?? 128000,
        temperature: 0.2,
        createdAt: now,
        updatedAt: now,
      })
      setPendingDeviceCode(null)
      track(config.completedEvent, { email: status.email })
      toast.success(`${config.displayName} Connected`, {
        description: status.email
          ? `Authenticated as ${status.email}`
          : `Successfully authenticated with ${config.displayName}`,
      })
    } catch (err) {
      toast.error(`Failed to create ${config.displayName} provider`, {
        description: err instanceof Error ? err.message : 'Unknown error',
      })
    } finally {
      flowStartedRef.current = false
    }
  }, [status?.authenticated])

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

  // Client-side: extension handles device code + polling, sends token to server
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

  // Server-side: server handles device code + polling
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

    // PKCE redirect flow
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
