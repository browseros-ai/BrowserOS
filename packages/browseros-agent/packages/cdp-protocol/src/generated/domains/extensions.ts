// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

// ══ Types ══

export type StorageArea = 'session' | 'local' | 'sync' | 'managed'

// ══ Commands ══

export interface LoadUnpackedParams {
  path: string
}

export interface LoadUnpackedResult {
  id: string
}

export interface UninstallParams {
  id: string
}

export interface GetStorageItemsParams {
  id: string
  storageArea: StorageArea
  keys?: string[]
}

export interface GetStorageItemsResult {
  data: Record<string, unknown>
}

export interface RemoveStorageItemsParams {
  id: string
  storageArea: StorageArea
  keys: string[]
}

export interface ClearStorageItemsParams {
  id: string
  storageArea: StorageArea
}

export interface SetStorageItemsParams {
  id: string
  storageArea: StorageArea
  values: Record<string, unknown>
}

// ── BrowserOS extension commands ──

export type ExtensionState = 'enabled' | 'disabled' | 'installed'

export interface ExtensionInfo {
  id: string
  name: string
  version: string
  description: string
  path: string
  state: ExtensionState
  isBrowserOS: boolean
  canModify: boolean
}

export interface ListExtensionsResult {
  extensions: ExtensionInfo[]
}

export interface GetExtensionInfoParams {
  id: string
}

export interface GetExtensionInfoResult {
  info: ExtensionInfo
}

export interface EnableExtensionParams {
  id: string
}

export interface DisableExtensionParams {
  id: string
}
