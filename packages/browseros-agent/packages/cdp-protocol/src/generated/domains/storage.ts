// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { BrowserContextID } from './browser'
import type { Cookie, CookieParam, TimeSinceEpoch } from './network'
import type { FrameId } from './page'

// ══ Types ══

export type SerializedStorageKey = string

export type StorageType =
  | 'cookies'
  | 'file_systems'
  | 'indexeddb'
  | 'local_storage'
  | 'shader_cache'
  | 'websql'
  | 'service_workers'
  | 'cache_storage'
  | 'storage_buckets'
  | 'all'
  | 'other'

export interface UsageForType {
  storageType: StorageType
  usage: number
}

export interface TrustTokens {
  issuerOrigin: string
  count: number
}

export type StorageBucketsDurability = 'relaxed' | 'strict'

export interface StorageBucket {
  storageKey: SerializedStorageKey
  name?: string
}

export interface StorageBucketInfo {
  bucket: StorageBucket
  id: string
  expiration: TimeSinceEpoch
  quota: number
  persistent: boolean
  durability: StorageBucketsDurability
}

export interface RelatedWebsiteSet {
  primarySites: string[]
  associatedSites: string[]
  serviceSites: string[]
}

// ══ Commands ══

export interface GetStorageKeyForFrameParams {
  frameId: FrameId
}

export interface GetStorageKeyForFrameResult {
  storageKey: SerializedStorageKey
}

export interface GetStorageKeyParams {
  frameId?: FrameId
}

export interface GetStorageKeyResult {
  storageKey: SerializedStorageKey
}

export interface ClearDataForOriginParams {
  origin: string
  storageTypes: string
}

export interface ClearDataForStorageKeyParams {
  storageKey: string
  storageTypes: string
}

export interface GetCookiesParams {
  browserContextId?: BrowserContextID
}

export interface GetCookiesResult {
  cookies: Cookie[]
}

export interface SetCookiesParams {
  cookies: CookieParam[]
  browserContextId?: BrowserContextID
}

export interface ClearCookiesParams {
  browserContextId?: BrowserContextID
}

export interface GetUsageAndQuotaParams {
  origin: string
}

export interface GetUsageAndQuotaResult {
  usage: number
  quota: number
  overrideActive: boolean
  usageBreakdown: UsageForType[]
}

export interface OverrideQuotaForOriginParams {
  origin: string
  quotaSize?: number
}

export interface TrackCacheStorageForOriginParams {
  origin: string
}

export interface TrackCacheStorageForStorageKeyParams {
  storageKey: string
}

export interface TrackIndexedDBForOriginParams {
  origin: string
}

export interface TrackIndexedDBForStorageKeyParams {
  storageKey: string
}

export interface UntrackCacheStorageForOriginParams {
  origin: string
}

export interface UntrackCacheStorageForStorageKeyParams {
  storageKey: string
}

export interface UntrackIndexedDBForOriginParams {
  origin: string
}

export interface UntrackIndexedDBForStorageKeyParams {
  storageKey: string
}

export interface GetTrustTokensResult {
  tokens: TrustTokens[]
}

export interface ClearTrustTokensParams {
  issuerOrigin: string
}

export interface ClearTrustTokensResult {
  didDeleteTokens: boolean
}

export interface SetStorageBucketTrackingParams {
  storageKey: string
  enable: boolean
}

export interface DeleteStorageBucketParams {
  bucket: StorageBucket
}

export interface RunBounceTrackingMitigationsResult {
  deletedSites: string[]
}

export interface GetRelatedWebsiteSetsResult {
  sets: RelatedWebsiteSet[]
}

// ══ Events ══

export interface CacheStorageContentUpdatedEvent {
  origin: string
  storageKey: string
  bucketId: string
  cacheName: string
}

export interface CacheStorageListUpdatedEvent {
  origin: string
  storageKey: string
  bucketId: string
}

export interface IndexedDBContentUpdatedEvent {
  origin: string
  storageKey: string
  bucketId: string
  databaseName: string
  objectStoreName: string
}

export interface IndexedDBListUpdatedEvent {
  origin: string
  storageKey: string
  bucketId: string
}

export interface StorageBucketCreatedOrUpdatedEvent {
  bucketInfo: StorageBucketInfo
}

export interface StorageBucketDeletedEvent {
  bucketId: string
}
