// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  CacheStorageContentUpdatedEvent,
  CacheStorageListUpdatedEvent,
  ClearCookiesParams,
  ClearDataForOriginParams,
  ClearDataForStorageKeyParams,
  ClearSharedStorageEntriesParams,
  ClearTrustTokensParams,
  ClearTrustTokensResult,
  DeleteSharedStorageEntryParams,
  DeleteStorageBucketParams,
  GetCookiesParams,
  GetCookiesResult,
  GetInterestGroupDetailsParams,
  GetInterestGroupDetailsResult,
  GetRelatedWebsiteSetsResult,
  GetSharedStorageEntriesParams,
  GetSharedStorageEntriesResult,
  GetSharedStorageMetadataParams,
  GetSharedStorageMetadataResult,
  GetStorageKeyForFrameParams,
  GetStorageKeyForFrameResult,
  GetStorageKeyParams,
  GetStorageKeyResult,
  GetTrustTokensResult,
  GetUsageAndQuotaParams,
  GetUsageAndQuotaResult,
  IndexedDBContentUpdatedEvent,
  IndexedDBListUpdatedEvent,
  InterestGroupAccessedEvent,
  InterestGroupAuctionEventOccurredEvent,
  InterestGroupAuctionNetworkRequestCreatedEvent,
  OverrideQuotaForOriginParams,
  ResetSharedStorageBudgetParams,
  RunBounceTrackingMitigationsResult,
  SetCookiesParams,
  SetInterestGroupAuctionTrackingParams,
  SetInterestGroupTrackingParams,
  SetProtectedAudienceKAnonymityParams,
  SetSharedStorageEntryParams,
  SetSharedStorageTrackingParams,
  SetStorageBucketTrackingParams,
  SharedStorageAccessedEvent,
  SharedStorageWorkletOperationExecutionFinishedEvent,
  StorageBucketCreatedOrUpdatedEvent,
  StorageBucketDeletedEvent,
  TrackCacheStorageForOriginParams,
  TrackCacheStorageForStorageKeyParams,
  TrackIndexedDBForOriginParams,
  TrackIndexedDBForStorageKeyParams,
  UntrackCacheStorageForOriginParams,
  UntrackCacheStorageForStorageKeyParams,
  UntrackIndexedDBForOriginParams,
  UntrackIndexedDBForStorageKeyParams,
} from '../domains/storage'

export interface StorageApi {
  // ── Commands ──

  getStorageKeyForFrame(
    params: GetStorageKeyForFrameParams,
  ): Promise<GetStorageKeyForFrameResult>
  getStorageKey(params?: GetStorageKeyParams): Promise<GetStorageKeyResult>
  clearDataForOrigin(params: ClearDataForOriginParams): Promise<void>
  clearDataForStorageKey(params: ClearDataForStorageKeyParams): Promise<void>
  getCookies(params?: GetCookiesParams): Promise<GetCookiesResult>
  setCookies(params: SetCookiesParams): Promise<void>
  clearCookies(params?: ClearCookiesParams): Promise<void>
  getUsageAndQuota(
    params: GetUsageAndQuotaParams,
  ): Promise<GetUsageAndQuotaResult>
  overrideQuotaForOrigin(params: OverrideQuotaForOriginParams): Promise<void>
  trackCacheStorageForOrigin(
    params: TrackCacheStorageForOriginParams,
  ): Promise<void>
  trackCacheStorageForStorageKey(
    params: TrackCacheStorageForStorageKeyParams,
  ): Promise<void>
  trackIndexedDBForOrigin(params: TrackIndexedDBForOriginParams): Promise<void>
  trackIndexedDBForStorageKey(
    params: TrackIndexedDBForStorageKeyParams,
  ): Promise<void>
  untrackCacheStorageForOrigin(
    params: UntrackCacheStorageForOriginParams,
  ): Promise<void>
  untrackCacheStorageForStorageKey(
    params: UntrackCacheStorageForStorageKeyParams,
  ): Promise<void>
  untrackIndexedDBForOrigin(
    params: UntrackIndexedDBForOriginParams,
  ): Promise<void>
  untrackIndexedDBForStorageKey(
    params: UntrackIndexedDBForStorageKeyParams,
  ): Promise<void>
  getTrustTokens(): Promise<GetTrustTokensResult>
  clearTrustTokens(
    params: ClearTrustTokensParams,
  ): Promise<ClearTrustTokensResult>
  getInterestGroupDetails(
    params: GetInterestGroupDetailsParams,
  ): Promise<GetInterestGroupDetailsResult>
  setInterestGroupTracking(
    params: SetInterestGroupTrackingParams,
  ): Promise<void>
  setInterestGroupAuctionTracking(
    params: SetInterestGroupAuctionTrackingParams,
  ): Promise<void>
  getSharedStorageMetadata(
    params: GetSharedStorageMetadataParams,
  ): Promise<GetSharedStorageMetadataResult>
  getSharedStorageEntries(
    params: GetSharedStorageEntriesParams,
  ): Promise<GetSharedStorageEntriesResult>
  setSharedStorageEntry(params: SetSharedStorageEntryParams): Promise<void>
  deleteSharedStorageEntry(
    params: DeleteSharedStorageEntryParams,
  ): Promise<void>
  clearSharedStorageEntries(
    params: ClearSharedStorageEntriesParams,
  ): Promise<void>
  resetSharedStorageBudget(
    params: ResetSharedStorageBudgetParams,
  ): Promise<void>
  setSharedStorageTracking(
    params: SetSharedStorageTrackingParams,
  ): Promise<void>
  setStorageBucketTracking(
    params: SetStorageBucketTrackingParams,
  ): Promise<void>
  deleteStorageBucket(params: DeleteStorageBucketParams): Promise<void>
  runBounceTrackingMitigations(): Promise<RunBounceTrackingMitigationsResult>
  getRelatedWebsiteSets(): Promise<GetRelatedWebsiteSetsResult>
  setProtectedAudienceKAnonymity(
    params: SetProtectedAudienceKAnonymityParams,
  ): Promise<void>

  // ── Events ──

  on(
    event: 'cacheStorageContentUpdated',
    handler: (params: CacheStorageContentUpdatedEvent) => void,
  ): () => void
  on(
    event: 'cacheStorageListUpdated',
    handler: (params: CacheStorageListUpdatedEvent) => void,
  ): () => void
  on(
    event: 'indexedDBContentUpdated',
    handler: (params: IndexedDBContentUpdatedEvent) => void,
  ): () => void
  on(
    event: 'indexedDBListUpdated',
    handler: (params: IndexedDBListUpdatedEvent) => void,
  ): () => void
  on(
    event: 'interestGroupAccessed',
    handler: (params: InterestGroupAccessedEvent) => void,
  ): () => void
  on(
    event: 'interestGroupAuctionEventOccurred',
    handler: (params: InterestGroupAuctionEventOccurredEvent) => void,
  ): () => void
  on(
    event: 'interestGroupAuctionNetworkRequestCreated',
    handler: (params: InterestGroupAuctionNetworkRequestCreatedEvent) => void,
  ): () => void
  on(
    event: 'sharedStorageAccessed',
    handler: (params: SharedStorageAccessedEvent) => void,
  ): () => void
  on(
    event: 'sharedStorageWorkletOperationExecutionFinished',
    handler: (
      params: SharedStorageWorkletOperationExecutionFinishedEvent,
    ) => void,
  ): () => void
  on(
    event: 'storageBucketCreatedOrUpdated',
    handler: (params: StorageBucketCreatedOrUpdatedEvent) => void,
  ): () => void
  on(
    event: 'storageBucketDeleted',
    handler: (params: StorageBucketDeletedEvent) => void,
  ): () => void
}
