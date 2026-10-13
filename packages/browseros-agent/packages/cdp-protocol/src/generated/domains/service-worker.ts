// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { TargetID } from './target'

// ══ Types ══

export type RegistrationID = string

export interface ServiceWorkerRegistration {
  registrationId: RegistrationID
  scopeURL: string
  isDeleted: boolean
}

export type ServiceWorkerVersionRunningStatus =
  | 'stopped'
  | 'starting'
  | 'running'
  | 'stopping'

export type ServiceWorkerVersionStatus =
  | 'new'
  | 'installing'
  | 'installed'
  | 'activating'
  | 'activated'
  | 'redundant'

export interface ServiceWorkerRouterCondition {
  urlPattern?: string
  requestMethod?: string
  requestMode?: string
  requestDestination?: string
  runningStatus?: ServiceWorkerVersionRunningStatus
  or?: ServiceWorkerRouterCondition[]
  not?: ServiceWorkerRouterCondition
}

export type ServiceWorkerRouterSourceType =
  | 'cache'
  | 'fetchEvent'
  | 'network'
  | 'raceNetworkAndFetchHandler'
  | 'raceNetworkAndCache'
  | 'sourceDict'

export interface ServiceWorkerRouterSourceDict {
  cacheName: string
}

export interface ServiceWorkerRouterSource {
  type: ServiceWorkerRouterSourceType
  sourceDict?: ServiceWorkerRouterSourceDict
}

export interface ServiceWorkerRouterRule {
  condition: ServiceWorkerRouterCondition
  source: ServiceWorkerRouterSource
  id: number
}

export interface ServiceWorkerVersion {
  versionId: string
  registrationId: RegistrationID
  scriptURL: string
  runningStatus: ServiceWorkerVersionRunningStatus
  status: ServiceWorkerVersionStatus
  scriptLastModified?: number
  scriptResponseTime?: number
  controlledClients?: TargetID[]
  targetId?: TargetID
  routerRules?: string
  typedRouterRules?: ServiceWorkerRouterRule[]
}

export interface ServiceWorkerErrorMessage {
  errorMessage: string
  registrationId: RegistrationID
  versionId: string
  sourceURL: string
  lineNumber: number
  columnNumber: number
}

// ══ Commands ══

export interface DeliverPushMessageParams {
  origin: string
  registrationId: RegistrationID
  data: string
}

export interface DispatchSyncEventParams {
  origin: string
  registrationId: RegistrationID
  tag: string
  lastChance: boolean
}

export interface DispatchPeriodicSyncEventParams {
  origin: string
  registrationId: RegistrationID
  tag: string
}

export interface SetForceUpdateOnPageLoadParams {
  forceUpdateOnPageLoad: boolean
}

export interface SkipWaitingParams {
  scopeURL: string
}

export interface StartWorkerParams {
  scopeURL: string
}

export interface StopWorkerParams {
  versionId: string
}

export interface UnregisterParams {
  scopeURL: string
}

export interface UpdateRegistrationParams {
  scopeURL: string
}

// ══ Events ══

export interface WorkerErrorReportedEvent {
  errorMessage: ServiceWorkerErrorMessage
}

export interface WorkerRegistrationUpdatedEvent {
  registrations: ServiceWorkerRegistration[]
}

export interface WorkerVersionUpdatedEvent {
  versions: ServiceWorkerVersion[]
}
