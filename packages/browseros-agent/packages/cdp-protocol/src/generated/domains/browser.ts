// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { FrameId } from './page'
import type { TargetID } from './target'

// ══ Types ══

export type BrowserContextID = string

export type WindowID = number

export type TabID = number

export type WindowType =
  | 'normal'
  | 'popup'
  | 'app'
  | 'devtools'
  | 'app_popup'
  | 'picture_in_picture'

export type WindowState = 'normal' | 'minimized' | 'maximized' | 'fullscreen'

export interface Bounds {
  left?: number
  top?: number
  width?: number
  height?: number
  windowState?: WindowState
}

export interface WindowInfo {
  windowId: WindowID
  windowType: WindowType
  bounds: Bounds
  isActive: boolean
  isVisible: boolean
  tabCount: number
  activeTabId?: TabID
  browserContextId?: BrowserContextID
}

export interface TabInfo {
  tabId: TabID
  targetId: TargetID
  url: string
  title: string
  isActive: boolean
  isLoading: boolean
  loadProgress: number
  isPinned: boolean
  isHidden: boolean
  windowId?: WindowID
  index?: number
  browserContextId?: BrowserContextID
  groupId?: TabGroupID
}

export type TabGroupID = string

export interface TabGroupInfo {
  groupId: TabGroupID
  windowId: WindowID
  title: string
  color: string
  collapsed: boolean
  tabIds: TabID[]
}

export type PermissionType =
  | 'ar'
  | 'audioCapture'
  | 'automaticFullscreen'
  | 'backgroundFetch'
  | 'backgroundSync'
  | 'cameraPanTiltZoom'
  | 'capturedSurfaceControl'
  | 'clipboardReadWrite'
  | 'clipboardSanitizedWrite'
  | 'displayCapture'
  | 'durableStorage'
  | 'geolocation'
  | 'handTracking'
  | 'idleDetection'
  | 'keyboardLock'
  | 'localFonts'
  | 'localNetwork'
  | 'localNetworkAccess'
  | 'loopbackNetwork'
  | 'midi'
  | 'midiSysex'
  | 'nfc'
  | 'notifications'
  | 'paymentHandler'
  | 'periodicBackgroundSync'
  | 'pointerLock'
  | 'protectedMediaIdentifier'
  | 'sensors'
  | 'smartCard'
  | 'speakerSelection'
  | 'storageAccess'
  | 'topLevelStorageAccess'
  | 'videoCapture'
  | 'vr'
  | 'wakeLockScreen'
  | 'wakeLockSystem'
  | 'webAppInstallation'
  | 'webPrinting'
  | 'windowManagement'

export type PermissionSetting = 'granted' | 'denied' | 'prompt'

export interface PermissionDescriptor {
  name: string
  sysex?: boolean
  userVisibleOnly?: boolean
  allowWithoutSanitization?: boolean
  allowWithoutGesture?: boolean
  panTiltZoom?: boolean
}

export type BrowserCommandId = 'openTabSearch' | 'closeTabSearch' | 'openGlic'

export interface Bucket {
  low: number
  high: number
  count: number
}

export interface Histogram {
  name: string
  sum: number
  count: number
  buckets: Bucket[]
}

export type PrivacySandboxAPI = 'BiddingAndAuctionServices' | 'TrustedKeyValue'

// ══ Commands ══

export interface GetWindowsResult {
  windows: WindowInfo[]
}

export interface GetActiveWindowResult {
  window?: WindowInfo
}

export interface CreateWindowParams {
  url?: string
  bounds?: Bounds
  windowType?: WindowType
  hidden?: boolean
  browserContextId?: BrowserContextID
}

export interface CreateWindowResult {
  window: WindowInfo
}

export interface CloseWindowParams {
  windowId: WindowID
}

export interface ActivateWindowParams {
  windowId: WindowID
}

export interface SetWindowVisibilityParams {
  windowId: WindowID
  visible: boolean
  activate?: boolean
}

export interface SetWindowVisibilityResult {
  window: WindowInfo
  replaced: boolean
  previousWindowId: WindowID
}

export interface GetTabsParams {
  windowId?: WindowID
  includeHidden?: boolean
}

export interface GetTabsResult {
  tabs: TabInfo[]
}

export interface GetActiveTabParams {
  windowId?: WindowID
}

export interface GetActiveTabResult {
  tab?: TabInfo
}

export interface GetTabInfoParams {
  targetId?: TargetID
  tabId?: TabID
}

export interface GetTabInfoResult {
  tab: TabInfo
}

export interface CreateTabParams {
  url?: string
  windowId?: WindowID
  index?: number
  background?: boolean
  pinned?: boolean
  browserContextId?: BrowserContextID
}

export interface CreateTabResult {
  tab: TabInfo
}

export interface CloseTabParams {
  targetId?: TargetID
  tabId?: TabID
}

export interface ActivateTabParams {
  targetId?: TargetID
  tabId?: TabID
}

export interface MoveTabParams {
  targetId?: TargetID
  tabId?: TabID
  windowId?: WindowID
  index?: number
}

export interface MoveTabResult {
  tab: TabInfo
}

export interface DuplicateTabParams {
  targetId?: TargetID
  tabId?: TabID
}

export interface DuplicateTabResult {
  tab: TabInfo
}

export interface PinTabParams {
  targetId?: TargetID
  tabId?: TabID
}

export interface PinTabResult {
  tab: TabInfo
}

export interface UnpinTabParams {
  targetId?: TargetID
  tabId?: TabID
}

export interface UnpinTabResult {
  tab: TabInfo
}

export interface ShowTabParams {
  targetId?: TargetID
  tabId?: TabID
  windowId?: WindowID
  index?: number
  activate?: boolean
}

export interface ShowTabResult {
  tab: TabInfo
}

export interface GetTabGroupsParams {
  windowId?: WindowID
}

export interface GetTabGroupsResult {
  groups: TabGroupInfo[]
}

export interface CreateTabGroupParams {
  tabIds: TabID[]
  title?: string
}

export interface CreateTabGroupResult {
  group: TabGroupInfo
}

export interface UpdateTabGroupParams {
  groupId: TabGroupID
  title?: string
  color?: string
  collapsed?: boolean
}

export interface UpdateTabGroupResult {
  group: TabGroupInfo
}

export interface CloseTabGroupParams {
  groupId: TabGroupID
}

export interface AddTabsToGroupParams {
  groupId: TabGroupID
  tabIds: TabID[]
}

export interface AddTabsToGroupResult {
  group: TabGroupInfo
}

export interface RemoveTabsFromGroupParams {
  tabIds: TabID[]
}

export interface MoveTabGroupParams {
  groupId: TabGroupID
  windowId?: WindowID
  index?: number
}

export interface MoveTabGroupResult {
  group: TabGroupInfo
}

export interface SetPermissionParams {
  permission: PermissionDescriptor
  setting: PermissionSetting
  origin?: string
  embeddedOrigin?: string
  browserContextId?: BrowserContextID
}

export interface GrantPermissionsParams {
  permissions: PermissionType[]
  origin?: string
  browserContextId?: BrowserContextID
}

export interface ResetPermissionsParams {
  browserContextId?: BrowserContextID
}

export interface SetDownloadBehaviorParams {
  behavior: 'deny' | 'allow' | 'allowAndName' | 'default'
  browserContextId?: BrowserContextID
  downloadPath?: string
  eventsEnabled?: boolean
}

export interface CancelDownloadParams {
  guid: string
  browserContextId?: BrowserContextID
}

export interface GetVersionResult {
  protocolVersion: string
  product: string
  revision: string
  userAgent: string
  jsVersion: string
}

export interface GetBrowserCommandLineResult {
  arguments: string[]
}

export interface GetHistogramsParams {
  query?: string
  delta?: boolean
}

export interface GetHistogramsResult {
  histograms: Histogram[]
}

export interface GetHistogramParams {
  name: string
  delta?: boolean
}

export interface GetHistogramResult {
  histogram: Histogram
}

export interface GetWindowBoundsParams {
  windowId: WindowID
}

export interface GetWindowBoundsResult {
  bounds: Bounds
}

export interface GetWindowForTargetParams {
  targetId?: TargetID
}

export interface GetWindowForTargetResult {
  windowId: WindowID
  bounds: Bounds
}

export interface GetTabForTargetParams {
  targetId?: TargetID
}

export interface GetTabForTargetResult {
  tabId: TabID
  windowId: WindowID
}

export interface GetTargetForTabParams {
  tabId: TabID
}

export interface GetTargetForTabResult {
  targetId: TargetID
  windowId: WindowID
}

export interface SetWindowBoundsParams {
  windowId: WindowID
  bounds: Bounds
}

export interface SetContentsSizeParams {
  windowId: WindowID
  width?: number
  height?: number
}

export interface SetDockTileParams {
  badgeLabel?: string
  image?: string
}

export interface ExecuteBrowserCommandParams {
  commandId: BrowserCommandId
}

export interface AddPrivacySandboxEnrollmentOverrideParams {
  url: string
}

export interface AddPrivacySandboxCoordinatorKeyConfigParams {
  api: PrivacySandboxAPI
  coordinatorOrigin: string
  keyConfig: string
  browserContextId?: BrowserContextID
}

// ══ Events ══

export interface DownloadWillBeginEvent {
  frameId: FrameId
  guid: string
  url: string
  suggestedFilename: string
}

export interface DownloadProgressEvent {
  guid: string
  totalBytes: number
  receivedBytes: number
  state: 'inProgress' | 'completed' | 'canceled'
  filePath?: string
}
