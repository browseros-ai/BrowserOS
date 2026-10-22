// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { BrowserContextID } from './browser'

// ══ Types ══

export type HistoryEntryID = string

export interface HistoryEntry {
  id: HistoryEntryID
  url: string
  title: string
  lastVisitTime: number
  visitCount: number
  typedCount: number
}

// ══ Commands ══

export interface SearchParams {
  query: string
  maxResults?: number
  startTime?: number
  endTime?: number
  browserContextId?: BrowserContextID
}

export interface SearchResult {
  entries: HistoryEntry[]
}

export interface GetRecentParams {
  maxResults?: number
  browserContextId?: BrowserContextID
}

export interface GetRecentResult {
  entries: HistoryEntry[]
}

export interface DeleteUrlParams {
  url: string
  browserContextId?: BrowserContextID
}

export interface DeleteRangeParams {
  startTime: number
  endTime: number
  browserContextId?: BrowserContextID
}
