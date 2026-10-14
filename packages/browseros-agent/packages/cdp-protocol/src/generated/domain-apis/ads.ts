// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { GetAdMetricsResult, GetAdScriptsResult } from '../domains/ads'

export interface AdsApi {
  // ── Commands ──

  getAdMetrics(): Promise<GetAdMetricsResult>
  getAdScripts(): Promise<GetAdScriptsResult>
}
