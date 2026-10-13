// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { AdProvenance } from './network'
import type { FrameId } from './page'
import type { ScriptId } from './runtime'

// ══ Types ══

export interface AdFrameData {
  frameId: FrameId
  initialOrigin?: string
  networkBytes: number
  cpuTime: number
}

export interface AdMetrics {
  viewportAdDensityByArea: number
  averageViewportAdDensityByArea: number
  viewportAdCount: number
  averageViewportAdCount: number
  totalAdCpuTime: number
  totalAdNetworkBytes: number
  updateAdFrames: AdFrameData[]
  removeAdFrames: FrameId[]
}

export interface AdScript {
  scriptId: ScriptId
  provenance: AdProvenance
}

// ══ Commands ══

export interface GetAdMetricsResult {
  metrics: AdMetrics
}

export interface GetAdScriptsResult {
  newScripts: AdScript[]
}
