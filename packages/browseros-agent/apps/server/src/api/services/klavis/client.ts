/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import { EXTERNAL_URLS } from '@browseros/shared/constants/urls'
import type { StrataCreateResponse, UserIntegration } from './types'

interface KlavisIntegrationObject {
  name?: string
  isAuthenticated?: boolean
  is_authenticated?: boolean
}

type KlavisIntegrationItem = string | KlavisIntegrationObject

/** Handles Klavis proxy HTTP calls and response normalization. */
export class KlavisClient {
  private baseUrl: string

  constructor(baseUrl?: string) {
    this.baseUrl = baseUrl || EXTERNAL_URLS.KLAVIS_PROXY
  }

  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<T> {
    const controller = new AbortController()
    const timeoutId = setTimeout(
      () => controller.abort(),
      TIMEOUTS.KLAVIS_FETCH,
    )

    try {
      const response = await fetch(`${this.baseUrl}${path}`, {
        method,
        headers: {
          'Content-Type': 'application/json',
        },
        body: body ? JSON.stringify(body) : undefined,
        signal: controller.signal,
      })

      if (!response.ok) {
        const errorText = await response.text()
        throw new Error(
          `Klavis error: ${response.status} ${response.statusText} - ${errorText}`,
        )
      }

      return response.json()
    } catch (error) {
      if (error instanceof Error && error.name === 'AbortError') {
        throw new Error(
          `Klavis request timed out after ${TIMEOUTS.KLAVIS_FETCH}ms`,
        )
      }
      throw error
    } finally {
      clearTimeout(timeoutId)
    }
  }

  /** Creates a Strata instance for the requested connector names. */
  async createStrata(
    userId: string,
    servers: string[],
  ): Promise<StrataCreateResponse> {
    return this.request<StrataCreateResponse>(
      'POST',
      '/mcp-server/strata/create',
      { userId, servers },
    )
  }

  /** Reads the user's Klavis integrations and normalizes auth flags. */
  async getUserIntegrations(userId: string): Promise<UserIntegration[]> {
    const data = await this.request<{
      integrations?: KlavisIntegrationItem[]
    }>('GET', `/user/${userId}/integrations`)
    const integrations = Array.isArray(data.integrations)
      ? data.integrations
      : []
    return integrations
      .map((integration) => this.normalizeIntegration(integration))
      .filter((integration): integration is UserIntegration =>
        Boolean(integration),
      )
  }

  private normalizeIntegration(
    integration: KlavisIntegrationItem,
  ): UserIntegration | null {
    if (typeof integration === 'string') {
      return { name: integration, isAuthenticated: false }
    }
    const name = integration.name
    if (!name || typeof name !== 'string') {
      return null
    }
    const isAuthenticated =
      typeof integration.isAuthenticated === 'boolean'
        ? integration.isAuthenticated
        : typeof integration.is_authenticated === 'boolean'
          ? integration.is_authenticated
          : false
    return { name, isAuthenticated }
  }

  /** Submits an API key through the Klavis instance auth endpoint. */
  async submitApiKey(apiKeyUrl: string, apiKey: string): Promise<void> {
    const parsedUrl = new URL(apiKeyUrl)
    const instanceId = parsedUrl.searchParams.get('instance_id')
    if (!instanceId) {
      throw new Error('Missing instance_id in apiKeyUrl')
    }

    await this.request('POST', '/mcp-server/instance/set-auth', {
      instanceId,
      authData: { api_key: apiKey },
    })
  }

  /** Removes connector names from an existing Strata instance. */
  async deleteServersFromStrata(
    strataId: string,
    servers: string[],
  ): Promise<void> {
    const query = servers.map(encodeURIComponent).join(',')
    await this.request(
      'DELETE',
      `/mcp-server/strata/${strataId}/servers?servers=${query}`,
    )
  }
}
