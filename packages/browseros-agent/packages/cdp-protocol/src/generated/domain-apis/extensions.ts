// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  ClearStorageItemsParams,
  DisableExtensionParams,
  EnableExtensionParams,
  GetExtensionInfoParams,
  GetExtensionInfoResult,
  GetStorageItemsParams,
  GetStorageItemsResult,
  ListExtensionsResult,
  LoadUnpackedParams,
  LoadUnpackedResult,
  RemoveStorageItemsParams,
  SetStorageItemsParams,
  UninstallParams,
} from '../domains/extensions'

export interface ExtensionsApi {
  // ── Commands ──

  loadUnpacked(params: LoadUnpackedParams): Promise<LoadUnpackedResult>
  uninstall(params: UninstallParams): Promise<void>
  getStorageItems(params: GetStorageItemsParams): Promise<GetStorageItemsResult>
  removeStorageItems(params: RemoveStorageItemsParams): Promise<void>
  clearStorageItems(params: ClearStorageItemsParams): Promise<void>
  setStorageItems(params: SetStorageItemsParams): Promise<void>

  // ── BrowserOS extension commands ──

  listExtensions(): Promise<ListExtensionsResult>
  getExtensionInfo(params: GetExtensionInfoParams): Promise<GetExtensionInfoResult>
  enableExtension(params: EnableExtensionParams): Promise<void>
  disableExtension(params: DisableExtensionParams): Promise<void>
}
