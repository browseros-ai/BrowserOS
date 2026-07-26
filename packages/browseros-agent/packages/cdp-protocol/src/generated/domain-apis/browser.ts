// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  ActivateTabParams,
  ActivateWindowParams,
  AddPrivacySandboxCoordinatorKeyConfigParams,
  AddPrivacySandboxEnrollmentOverrideParams,
  AddTabsToGroupParams,
  AddTabsToGroupResult,
  CancelDownloadParams,
  CloseTabGroupParams,
  CloseTabParams,
  CloseWindowParams,
  CreateTabGroupParams,
  CreateTabGroupResult,
  CreateTabParams,
  CreateTabResult,
  CreateWindowParams,
  CreateWindowResult,
  DownloadProgressEvent,
  DownloadWillBeginEvent,
  DuplicateTabParams,
  DuplicateTabResult,
  ExecuteBrowserCommandParams,
  GetActiveTabParams,
  GetActiveTabResult,
  GetActiveWindowResult,
  GetBrowserCommandLineResult,
  GetHistogramParams,
  GetHistogramResult,
  GetHistogramsParams,
  GetHistogramsResult,
  GetTabForTargetParams,
  GetTabForTargetResult,
  GetTabGroupsParams,
  GetTabGroupsResult,
  GetTabInfoParams,
  GetTabInfoResult,
  GetTabsParams,
  GetTabsResult,
  GetTargetForTabParams,
  GetTargetForTabResult,
  GetVersionResult,
  GetWindowBoundsParams,
  GetWindowBoundsResult,
  GetWindowForTargetParams,
  GetWindowForTargetResult,
  GetWindowsResult,
  GrantPermissionsParams,
  MoveTabGroupParams,
  MoveTabGroupResult,
  MoveTabParams,
  MoveTabResult,
  PinTabParams,
  PinTabResult,
  RemoveTabsFromGroupParams,
  ResetPermissionsParams,
  SetContentsSizeParams,
  SetDockTileParams,
  SetDownloadBehaviorParams,
  SetPermissionParams,
  SetWindowBoundsParams,
  SetWindowVisibilityParams,
  SetWindowVisibilityResult,
  ShowTabParams,
  ShowTabResult,
  UnpinTabParams,
  UnpinTabResult,
  UpdateTabGroupParams,
  UpdateTabGroupResult,
} from '../domains/browser'

export interface BrowserApi {
  // ── Commands ──

  getWindows(): Promise<GetWindowsResult>
  getActiveWindow(): Promise<GetActiveWindowResult>
  createWindow(params?: CreateWindowParams): Promise<CreateWindowResult>
  closeWindow(params: CloseWindowParams): Promise<void>
  activateWindow(params: ActivateWindowParams): Promise<void>
  setWindowVisibility(
    params: SetWindowVisibilityParams,
  ): Promise<SetWindowVisibilityResult>
  getTabs(params?: GetTabsParams): Promise<GetTabsResult>
  getActiveTab(params?: GetActiveTabParams): Promise<GetActiveTabResult>
  getTabInfo(params?: GetTabInfoParams): Promise<GetTabInfoResult>
  createTab(params?: CreateTabParams): Promise<CreateTabResult>
  closeTab(params?: CloseTabParams): Promise<void>
  activateTab(params?: ActivateTabParams): Promise<void>
  moveTab(params?: MoveTabParams): Promise<MoveTabResult>
  duplicateTab(params?: DuplicateTabParams): Promise<DuplicateTabResult>
  pinTab(params?: PinTabParams): Promise<PinTabResult>
  unpinTab(params?: UnpinTabParams): Promise<UnpinTabResult>
  showTab(params?: ShowTabParams): Promise<ShowTabResult>
  getTabGroups(params?: GetTabGroupsParams): Promise<GetTabGroupsResult>
  createTabGroup(params: CreateTabGroupParams): Promise<CreateTabGroupResult>
  updateTabGroup(params: UpdateTabGroupParams): Promise<UpdateTabGroupResult>
  closeTabGroup(params: CloseTabGroupParams): Promise<void>
  addTabsToGroup(params: AddTabsToGroupParams): Promise<AddTabsToGroupResult>
  removeTabsFromGroup(params: RemoveTabsFromGroupParams): Promise<void>
  moveTabGroup(params: MoveTabGroupParams): Promise<MoveTabGroupResult>
  setPermission(params: SetPermissionParams): Promise<void>
  grantPermissions(params: GrantPermissionsParams): Promise<void>
  resetPermissions(params?: ResetPermissionsParams): Promise<void>
  setDownloadBehavior(params: SetDownloadBehaviorParams): Promise<void>
  cancelDownload(params: CancelDownloadParams): Promise<void>
  close(): Promise<void>
  crash(): Promise<void>
  crashGpuProcess(): Promise<void>
  getVersion(): Promise<GetVersionResult>
  getBrowserCommandLine(): Promise<GetBrowserCommandLineResult>
  getHistograms(params?: GetHistogramsParams): Promise<GetHistogramsResult>
  getHistogram(params: GetHistogramParams): Promise<GetHistogramResult>
  getWindowBounds(params: GetWindowBoundsParams): Promise<GetWindowBoundsResult>
  getWindowForTarget(
    params?: GetWindowForTargetParams,
  ): Promise<GetWindowForTargetResult>
  getTabForTarget(
    params?: GetTabForTargetParams,
  ): Promise<GetTabForTargetResult>
  getTargetForTab(params: GetTargetForTabParams): Promise<GetTargetForTabResult>
  setWindowBounds(params: SetWindowBoundsParams): Promise<void>
  setContentsSize(params: SetContentsSizeParams): Promise<void>
  setDockTile(params?: SetDockTileParams): Promise<void>
  executeBrowserCommand(params: ExecuteBrowserCommandParams): Promise<void>
  addPrivacySandboxEnrollmentOverride(
    params: AddPrivacySandboxEnrollmentOverrideParams,
  ): Promise<void>
  addPrivacySandboxCoordinatorKeyConfig(
    params: AddPrivacySandboxCoordinatorKeyConfigParams,
  ): Promise<void>

  // ── Events ──

  on(
    event: 'downloadWillBegin',
    handler: (params: DownloadWillBeginEvent) => void,
  ): () => void
  on(
    event: 'downloadProgress',
    handler: (params: DownloadProgressEvent) => void,
  ): () => void
}
