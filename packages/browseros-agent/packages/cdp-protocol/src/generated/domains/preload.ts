// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { BackendNodeId } from './dom'
import type { LoaderId, RequestId } from './network'
import type { FrameId } from './page'

// ══ Types ══

export type RuleSetId = string

export interface RuleSet {
  id: RuleSetId
  loaderId: LoaderId
  sourceText: string
  backendNodeId?: BackendNodeId
  url?: string
  requestId?: RequestId
  errorType?: RuleSetErrorType
  errorMessage?: string
  tag?: string
}

export type RuleSetErrorType =
  | 'SourceIsNotJsonObject'
  | 'InvalidRulesSkipped'
  | 'InvalidRulesetLevelTag'

export type SpeculationAction =
  | 'Prefetch'
  | 'Prerender'
  | 'PrerenderUntilScript'

export type SpeculationTargetHint = 'Blank' | 'Self'

export interface PreloadingAttemptKey {
  loaderId: LoaderId
  action: SpeculationAction
  url: string
  formSubmission?: boolean
  targetHint?: SpeculationTargetHint
}

export interface PreloadingAttemptSource {
  key: PreloadingAttemptKey
  ruleSetIds: RuleSetId[]
  nodeIds: BackendNodeId[]
}

export type PreloadPipelineId = string

export type PrerenderFinalStatus =
  | 'Activated'
  | 'Destroyed'
  | 'LowEndDevice'
  | 'InvalidSchemeRedirect'
  | 'InvalidSchemeNavigation'
  | 'NavigationRequestBlockedByCsp'
  | 'MojoBinderPolicy'
  | 'RendererProcessCrashed'
  | 'RendererProcessKilled'
  | 'Download'
  | 'TriggerDestroyed'
  | 'NavigationNotCommitted'
  | 'NavigationBadHttpStatus'
  | 'ClientCertRequested'
  | 'NavigationRequestNetworkError'
  | 'CancelAllHostsForTesting'
  | 'DidFailLoad'
  | 'Stop'
  | 'SslCertificateError'
  | 'LoginAuthRequested'
  | 'UaChangeRequiresReload'
  | 'BlockedByClient'
  | 'AudioOutputDeviceRequested'
  | 'MixedContent'
  | 'TriggerBackgrounded'
  | 'MemoryLimitExceeded'
  | 'DataSaverEnabled'
  | 'TriggerUrlHasEffectiveUrl'
  | 'ActivatedBeforeStarted'
  | 'InactivePageRestriction'
  | 'StartFailed'
  | 'TimeoutBackgrounded'
  | 'CrossSiteRedirectInInitialNavigation'
  | 'CrossSiteNavigationInInitialNavigation'
  | 'SameSiteCrossOriginRedirectNotOptInInInitialNavigation'
  | 'SameSiteCrossOriginNavigationNotOptInInInitialNavigation'
  | 'ActivationNavigationParameterMismatch'
  | 'ActivatedInBackground'
  | 'EmbedderHostDisallowed'
  | 'ActivationNavigationDestroyedBeforeSuccess'
  | 'TabClosedByUserGesture'
  | 'TabClosedWithoutUserGesture'
  | 'PrimaryMainFrameRendererProcessCrashed'
  | 'PrimaryMainFrameRendererProcessKilled'
  | 'ActivationFramePolicyNotCompatible'
  | 'PreloadingDisabled'
  | 'BatterySaverEnabled'
  | 'ActivatedDuringMainFrameNavigation'
  | 'PreloadingUnsupportedByWebContents'
  | 'CrossSiteRedirectInMainFrameNavigation'
  | 'CrossSiteNavigationInMainFrameNavigation'
  | 'SameSiteCrossOriginRedirectNotOptInInMainFrameNavigation'
  | 'SameSiteCrossOriginNavigationNotOptInInMainFrameNavigation'
  | 'MemoryPressureOnTrigger'
  | 'MemoryPressureAfterTriggered'
  | 'PrerenderingDisabledByDevTools'
  | 'SpeculationRuleRemoved'
  | 'ActivatedWithAuxiliaryBrowsingContexts'
  | 'MaxNumOfRunningEagerPrerendersExceeded'
  | 'MaxNumOfRunningNonEagerPrerendersExceeded'
  | 'MaxNumOfRunningEmbedderPrerendersExceeded'
  | 'PrerenderingUrlHasEffectiveUrl'
  | 'RedirectedPrerenderingUrlHasEffectiveUrl'
  | 'ActivationUrlHasEffectiveUrl'
  | 'JavaScriptInterfaceAdded'
  | 'JavaScriptInterfaceRemoved'
  | 'AllPrerenderingCanceled'
  | 'WindowClosed'
  | 'SlowNetwork'
  | 'OtherPrerenderedPageActivated'
  | 'V8OptimizerDisabled'
  | 'PrerenderFailedDuringPrefetch'
  | 'BrowsingDataRemoved'
  | 'PrerenderHostReused'
  | 'FormSubmitWhenPrerendering'

export type PreloadingStatus =
  | 'Pending'
  | 'Running'
  | 'Ready'
  | 'Success'
  | 'Failure'
  | 'NotSupported'

export type PrefetchStatus =
  | 'PrefetchAllowed'
  | 'PrefetchFailedIneligibleRedirect'
  | 'PrefetchFailedInvalidRedirect'
  | 'PrefetchFailedMIMENotSupported'
  | 'PrefetchFailedNetError'
  | 'PrefetchFailedNon2XX'
  | 'PrefetchEvictedAfterBrowsingDataRemoved'
  | 'PrefetchEvictedAfterCandidateRemoved'
  | 'PrefetchEvictedForNewerPrefetch'
  | 'PrefetchHeldback'
  | 'PrefetchIneligibleRetryAfter'
  | 'PrefetchIsPrivacyDecoy'
  | 'PrefetchIsStale'
  | 'PrefetchNotEligibleBrowserContextOffTheRecord'
  | 'PrefetchNotEligibleDataSaverEnabled'
  | 'PrefetchNotEligibleExistingProxy'
  | 'PrefetchNotEligibleHostIsNonUnique'
  | 'PrefetchNotEligibleNonDefaultStoragePartition'
  | 'PrefetchNotEligibleSameSiteCrossOriginPrefetchRequiredProxy'
  | 'PrefetchNotEligibleSchemeIsNotHttps'
  | 'PrefetchNotEligibleUserHasCookies'
  | 'PrefetchNotEligibleUserHasServiceWorker'
  | 'PrefetchNotEligibleUserHasServiceWorkerNoFetchHandler'
  | 'PrefetchNotEligibleRedirectFromServiceWorker'
  | 'PrefetchNotEligibleRedirectToServiceWorker'
  | 'PrefetchNotEligibleBatterySaverEnabled'
  | 'PrefetchNotEligiblePreloadingDisabled'
  | 'PrefetchNotFinishedInTime'
  | 'PrefetchNotStarted'
  | 'PrefetchNotUsedCookiesChanged'
  | 'PrefetchProxyNotAvailable'
  | 'PrefetchResponseUsed'
  | 'PrefetchSuccessfulButNotUsed'
  | 'PrefetchNotUsedProbeFailed'

export interface PrerenderMismatchedHeaders {
  headerName: string
  initialValue?: string
  activationValue?: string
}

// ══ Commands ══

// ══ Events ══

export interface RuleSetUpdatedEvent {
  ruleSet: RuleSet
}

export interface RuleSetRemovedEvent {
  id: RuleSetId
}

export interface PreloadEnabledStateUpdatedEvent {
  disabledByPreference: boolean
  disabledByDataSaver: boolean
  disabledByBatterySaver: boolean
  disabledByHoldbackPrefetchSpeculationRules: boolean
  disabledByHoldbackPrerenderSpeculationRules: boolean
}

export interface PrefetchStatusUpdatedEvent {
  key: PreloadingAttemptKey
  pipelineId: PreloadPipelineId
  initiatingFrameId: FrameId
  prefetchUrl: string
  status: PreloadingStatus
  prefetchStatus: PrefetchStatus
  requestId: RequestId
}

export interface PrerenderStatusUpdatedEvent {
  key: PreloadingAttemptKey
  pipelineId: PreloadPipelineId
  status: PreloadingStatus
  prerenderStatus?: PrerenderFinalStatus
  disallowedMojoInterface?: string
  mismatchedHeaders?: PrerenderMismatchedHeaders[]
}

export interface PreloadingAttemptSourcesUpdatedEvent {
  loaderId: LoaderId
  preloadingAttemptSources: PreloadingAttemptSource[]
}
