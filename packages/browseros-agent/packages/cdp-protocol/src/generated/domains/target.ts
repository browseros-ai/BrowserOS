// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { BrowserContextID, TabID, WindowID } from './browser'
import type { FrameId } from './page'

// ══ Types ══

export type TargetID = string

export type SessionID = string

export interface TargetInfo {
  targetId: TargetID
  type: string
  title: string
  url: string
  attached: boolean
  openerId?: TargetID
  canAccessOpener: boolean
  openerFrameId?: FrameId
  parentFrameId?: FrameId
  browserContextId?: BrowserContextID
  subtype?: string
  tabId?: TabID
  windowId?: WindowID
}

export interface FilterEntry {
  exclude?: boolean
  type?: string
}

export type TargetFilter = FilterEntry[]

export interface RemoteLocation {
  host: string
  port: number
}

export type WindowState = 'normal' | 'minimized' | 'maximized' | 'fullscreen'

// ══ Commands ══

export interface ActivateTargetParams {
  targetId: TargetID
}

export interface AttachToTargetParams {
  targetId: TargetID
  flatten?: boolean
}

export interface AttachToTargetResult {
  sessionId: SessionID
}

export interface AttachToBrowserTargetResult {
  sessionId: SessionID
}

export interface CloseTargetParams {
  targetId: TargetID
}

export interface CloseTargetResult {
  success: boolean
}

export interface ExposeDevToolsProtocolParams {
  targetId: TargetID
  bindingName?: string
  inheritPermissions?: boolean
}

export interface CreateBrowserContextParams {
  disposeOnDetach?: boolean
  proxyServer?: string
  proxyBypassList?: string
  originsWithUniversalNetworkAccess?: string[]
}

export interface CreateBrowserContextResult {
  browserContextId: BrowserContextID
}

export interface GetBrowserContextsResult {
  browserContextIds: BrowserContextID[]
  defaultBrowserContextId?: BrowserContextID
}

export interface CreateTargetParams {
  url: string
  left?: number
  top?: number
  width?: number
  height?: number
  windowState?: WindowState
  browserContextId?: BrowserContextID
  enableBeginFrameControl?: boolean
  newWindow?: boolean
  background?: boolean
  forTab?: boolean
  hidden?: boolean
  focus?: boolean
}

export interface CreateTargetResult {
  targetId: TargetID
}

export interface DetachFromTargetParams {
  sessionId?: SessionID
  targetId?: TargetID
}

export interface DisposeBrowserContextParams {
  browserContextId: BrowserContextID
}

export interface GetTargetInfoParams {
  targetId?: TargetID
}

export interface GetTargetInfoResult {
  targetInfo: TargetInfo
}

export interface GetTargetsParams {
  filter?: TargetFilter
}

export interface GetTargetsResult {
  targetInfos: TargetInfo[]
}

export interface SendMessageToTargetParams {
  message: string
  sessionId?: SessionID
  targetId?: TargetID
}

export interface SetAutoAttachParams {
  autoAttach: boolean
  waitForDebuggerOnStart: boolean
  flatten?: boolean
  filter?: TargetFilter
}

export interface AutoAttachRelatedParams {
  targetId: TargetID
  waitForDebuggerOnStart: boolean
  filter?: TargetFilter
}

export interface SetDiscoverTargetsParams {
  discover: boolean
  filter?: TargetFilter
}

export interface SetRemoteLocationsParams {
  locations: RemoteLocation[]
}

export interface GetDevToolsTargetParams {
  targetId: TargetID
}

export interface GetDevToolsTargetResult {
  targetId?: TargetID
}

export interface OpenDevToolsParams {
  targetId: TargetID
  panelId?: string
}

export interface OpenDevToolsResult {
  targetId: TargetID
}

// ══ Events ══

export interface AttachedToTargetEvent {
  sessionId: SessionID
  targetInfo: TargetInfo
  waitingForDebugger: boolean
}

export interface DetachedFromTargetEvent {
  sessionId: SessionID
  targetId?: TargetID
}

export interface ReceivedMessageFromTargetEvent {
  sessionId: SessionID
  message: string
  targetId?: TargetID
}

export interface TargetCreatedEvent {
  targetInfo: TargetInfo
}

export interface TargetDestroyedEvent {
  targetId: TargetID
}

export interface TargetCrashedEvent {
  targetId: TargetID
  status: string
  errorCode: number
}

export interface TargetInfoChangedEvent {
  targetInfo: TargetInfo
}
