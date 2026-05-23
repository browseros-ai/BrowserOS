/**
 * @license
 * Copyright 2025 BrowserOS
 */
import type { PostHog } from 'posthog-node'

const EVENT_PREFIX = 'browseros.server.'

export interface MetricsConfig {
  client_id?: string
  install_id?: string
  browseros_version?: string
  chromium_version?: string
  server_version?: string
  [key: string]: string | undefined
}

class MetricsService {
  private client: PostHog | null = null
  private config: MetricsConfig | null = null

  initialize(config: MetricsConfig): void {
    this.config = { ...this.config, ...config }
    // Security Hardening: PostHog disabled on server side to prevent data leakage and network noise
    this.client = null
  }

  isEnabled(): boolean {
    return false
  }

  getClientId(): string | null {
    return this.config?.client_id ?? null
  }

  log(eventName: string, properties: Record<string, unknown> = {}): void {
    if (!this.client || !this.config) {
      return
    }

    const {
      client_id,
      install_id,
      browseros_version,
      chromium_version,
      server_version,
      ...defaultProperties
    } = this.config

    const distinctId = client_id || install_id || 'anonymous'

    this.client.capture({
      distinctId,
      event: EVENT_PREFIX + eventName,
      properties: {
        ...defaultProperties,
        ...properties,
        ...(client_id && { client_id }),
        ...(install_id && { install_id }),
        ...(browseros_version && { browseros_version }),
        ...(chromium_version && { chromium_version }),
        ...(server_version && { server_version }),
        $process_person_profile: false,
      },
    })
  }

  async shutdown(): Promise<void> {
    if (this.client) {
      await this.client.shutdown()
      this.client = null
    }
  }
}

export const metrics = new MetricsService()
