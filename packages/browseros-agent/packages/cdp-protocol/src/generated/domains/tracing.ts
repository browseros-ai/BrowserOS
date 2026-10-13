// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { StreamHandle } from './io'

// ══ Types ══

export type MemoryDumpConfig = Record<string, unknown>

export interface TraceConfig {
  recordMode?:
    | 'recordUntilFull'
    | 'recordContinuously'
    | 'recordAsMuchAsPossible'
    | 'echoToConsole'
  traceBufferSizeInKb?: number
  enableSampling?: boolean
  enableSystrace?: boolean
  enableArgumentFilter?: boolean
  includedCategories?: string[]
  excludedCategories?: string[]
  syntheticDelays?: string[]
  memoryDumpConfig?: MemoryDumpConfig
}

export type StreamFormat = 'json' | 'proto'

export type StreamCompression = 'none' | 'gzip'

export type MemoryDumpLevelOfDetail = 'background' | 'light' | 'detailed'

export type TracingBackend = 'auto' | 'chrome' | 'system'

// ══ Commands ══

export interface GetCategoriesResult {
  categories: string[]
}

export interface GetTrackEventDescriptorResult {
  descriptor: string
}

export interface RecordClockSyncMarkerParams {
  syncId: string
}

export interface RequestMemoryDumpParams {
  deterministic?: boolean
  levelOfDetail?: MemoryDumpLevelOfDetail
}

export interface RequestMemoryDumpResult {
  dumpGuid: string
  success: boolean
}

export interface StartParams {
  categories?: string
  options?: string
  bufferUsageReportingInterval?: number
  transferMode?: 'ReportEvents' | 'ReturnAsStream'
  streamFormat?: StreamFormat
  streamCompression?: StreamCompression
  traceConfig?: TraceConfig
  perfettoConfig?: string
  tracingBackend?: TracingBackend
  screenshotMaxSize?: number
  screenshotMaxCount?: number
}

// ══ Events ══

export interface BufferUsageEvent {
  percentFull?: number
  eventCount?: number
  value?: number
}

export interface DataCollectedEvent {
  value: Record<string, unknown>[]
}

export interface TracingCompleteEvent {
  dataLossOccurred: boolean
  stream?: StreamHandle
  traceFormat?: StreamFormat
  streamCompression?: StreamCompression
}
