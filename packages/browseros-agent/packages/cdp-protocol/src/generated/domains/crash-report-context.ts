// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { FrameId } from './page'

// ══ Types ══

export interface CrashReportContextEntry {
  key: string
  value: string
  frameId: FrameId
}

// ══ Commands ══

export interface GetEntriesResult {
  entries: CrashReportContextEntry[]
}
