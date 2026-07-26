// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { SearchMatch } from './debugger'
import type { BackendNodeId, Rect } from './dom'
import type { ScreenOrientation } from './emulation'
import type { StreamHandle } from './io'
import type {
  AdAncestry,
  LoaderId,
  MonotonicTime,
  ResourceType,
  TimeSinceEpoch,
} from './network'
import type { ExecutionContextId, StackTrace } from './runtime'

// ══ Types ══

export type FrameId = string

export type AdFrameType = 'none' | 'child' | 'root'

export type AdFrameExplanation =
  | 'ParentIsAd'
  | 'CreatedByAdScript'
  | 'MatchedBlockingRule'

export interface AdFrameStatus {
  adFrameType: AdFrameType
  explanations?: AdFrameExplanation[]
}

export type SecureContextType =
  | 'Secure'
  | 'SecureLocalhost'
  | 'InsecureScheme'
  | 'InsecureAncestor'

export type CrossOriginIsolatedContextType =
  | 'Isolated'
  | 'NotIsolated'
  | 'NotIsolatedFeatureDisabled'

export type GatedAPIFeatures =
  | 'SharedArrayBuffers'
  | 'SharedArrayBuffersTransferAllowed'
  | 'PerformanceMeasureMemory'
  | 'PerformanceProfile'

export type PermissionsPolicyFeature =
  | 'accelerometer'
  | 'all-screens-capture'
  | 'ambient-light-sensor'
  | 'aria-notify'
  | 'attribution-reporting'
  | 'autofill'
  | 'autoplay'
  | 'bluetooth'
  | 'browsing-topics'
  | 'camera'
  | 'captured-surface-control'
  | 'ch-dpr'
  | 'ch-device-memory'
  | 'ch-downlink'
  | 'ch-ect'
  | 'ch-prefers-color-scheme'
  | 'ch-prefers-reduced-motion'
  | 'ch-prefers-reduced-transparency'
  | 'ch-rtt'
  | 'ch-save-data'
  | 'ch-ua'
  | 'ch-ua-arch'
  | 'ch-ua-bitness'
  | 'ch-ua-high-entropy-values'
  | 'ch-ua-platform'
  | 'ch-ua-model'
  | 'ch-ua-mobile'
  | 'ch-ua-form-factors'
  | 'ch-ua-full-version'
  | 'ch-ua-full-version-list'
  | 'ch-ua-platform-version'
  | 'ch-ua-wow64'
  | 'ch-viewport-height'
  | 'ch-viewport-width'
  | 'ch-width'
  | 'clipboard-read'
  | 'clipboard-write'
  | 'compute-pressure'
  | 'controlled-frame'
  | 'cross-origin-isolated'
  | 'deferred-fetch'
  | 'deferred-fetch-minimal'
  | 'device-attributes'
  | 'digital-credentials-create'
  | 'digital-credentials-get'
  | 'direct-sockets'
  | 'direct-sockets-multicast'
  | 'direct-sockets-private'
  | 'display-capture'
  | 'document-domain'
  | 'encrypted-media'
  | 'execution-while-out-of-viewport'
  | 'execution-while-not-rendered'
  | 'focus-without-user-activation'
  | 'fullscreen'
  | 'frobulate'
  | 'gamepad'
  | 'geolocation'
  | 'gyroscope'
  | 'hid'
  | 'identity-credentials-get'
  | 'idle-detection'
  | 'interest-cohort'
  | 'join-ad-interest-group'
  | 'keyboard-map'
  | 'language-detector'
  | 'language-model'
  | 'local-fonts'
  | 'local-network'
  | 'local-network-access'
  | 'loopback-network'
  | 'magnetometer'
  | 'manual-text'
  | 'media-playback-while-not-visible'
  | 'microphone'
  | 'midi'
  | 'on-device-speech-recognition'
  | 'otp-credentials'
  | 'payment'
  | 'picture-in-picture'
  | 'private-aggregation'
  | 'private-state-token-issuance'
  | 'private-state-token-redemption'
  | 'publickey-credentials-create'
  | 'publickey-credentials-get'
  | 'record-ad-auction-events'
  | 'rewriter'
  | 'run-ad-auction'
  | 'screen-wake-lock'
  | 'serial'
  | 'shared-storage'
  | 'shared-storage-select-url'
  | 'smart-card'
  | 'speaker-selection'
  | 'storage-access'
  | 'sub-apps'
  | 'summarizer'
  | 'sync-xhr'
  | 'translator'
  | 'unload'
  | 'usb'
  | 'usb-unrestricted'
  | 'vertical-scroll'
  | 'web-app-installation'
  | 'web-printing'
  | 'web-share'
  | 'window-management'
  | 'writer'
  | 'xr-spatial-tracking'

export type PermissionsPolicyBlockReason =
  | 'Header'
  | 'IframeAttribute'
  | 'InFencedFrameTree'
  | 'InIsolatedApp'

export interface PermissionsPolicyBlockLocator {
  frameId: FrameId
  blockReason: PermissionsPolicyBlockReason
}

export interface PermissionsPolicyFeatureState {
  feature: PermissionsPolicyFeature
  allowed: boolean
  locator?: PermissionsPolicyBlockLocator
}

export type OriginTrialTokenStatus =
  | 'Success'
  | 'NotSupported'
  | 'Insecure'
  | 'Expired'
  | 'WrongOrigin'
  | 'InvalidSignature'
  | 'Malformed'
  | 'WrongVersion'
  | 'FeatureDisabled'
  | 'TokenDisabled'
  | 'FeatureDisabledForUser'
  | 'UnknownTrial'

export type OriginTrialStatus =
  | 'Enabled'
  | 'ValidTokenNotProvided'
  | 'OSNotSupported'
  | 'TrialNotAllowed'

export type OriginTrialUsageRestriction = 'None' | 'Subset'

export interface OriginTrialToken {
  origin: string
  matchSubDomains: boolean
  trialName: string
  expiryTime: TimeSinceEpoch
  isThirdParty: boolean
  usageRestriction: OriginTrialUsageRestriction
}

export interface OriginTrialTokenWithStatus {
  rawTokenText: string
  parsedToken?: OriginTrialToken
  status: OriginTrialTokenStatus
}

export interface OriginTrial {
  trialName: string
  status: OriginTrialStatus
  tokensWithStatus: OriginTrialTokenWithStatus[]
}

export interface SecurityOriginDetails {
  isLocalhost: boolean
}

export interface Frame {
  id: FrameId
  parentId?: FrameId
  loaderId: LoaderId
  name?: string
  url: string
  urlFragment?: string
  domainAndRegistry: string
  securityOrigin: string
  securityOriginDetails?: SecurityOriginDetails
  mimeType: string
  unreachableUrl?: string
  adFrameStatus?: AdFrameStatus
  secureContextType: SecureContextType
  crossOriginIsolatedContextType: CrossOriginIsolatedContextType
  gatedAPIFeatures: GatedAPIFeatures[]
}

export interface FrameResource {
  url: string
  type: ResourceType
  mimeType: string
  lastModified?: TimeSinceEpoch
  contentSize?: number
  failed?: boolean
  canceled?: boolean
}

export interface FrameResourceTree {
  frame: Frame
  childFrames?: FrameResourceTree[]
  resources: FrameResource[]
}

export interface FrameTree {
  frame: Frame
  childFrames?: FrameTree[]
}

export type ScriptIdentifier = string

export type TransitionType =
  | 'link'
  | 'typed'
  | 'address_bar'
  | 'auto_bookmark'
  | 'auto_subframe'
  | 'manual_subframe'
  | 'generated'
  | 'auto_toplevel'
  | 'form_submit'
  | 'reload'
  | 'keyword'
  | 'keyword_generated'
  | 'other'

export interface NavigationEntry {
  id: number
  url: string
  userTypedURL: string
  title: string
  transitionType: TransitionType
}

export interface ScreencastFrameMetadata {
  offsetTop: number
  pageScaleFactor: number
  deviceWidth: number
  deviceHeight: number
  scrollOffsetX: number
  scrollOffsetY: number
  timestamp?: TimeSinceEpoch
}

export type DialogType = 'alert' | 'confirm' | 'prompt' | 'beforeunload'

export interface AppManifestError {
  message: string
  critical: number
  line: number
  column: number
}

export interface AppManifestParsedProperties {
  scope: string
}

export interface LayoutViewport {
  pageX: number
  pageY: number
  clientWidth: number
  clientHeight: number
}

export interface VisualViewport {
  offsetX: number
  offsetY: number
  pageX: number
  pageY: number
  clientWidth: number
  clientHeight: number
  scale: number
  zoom?: number
}

export interface Viewport {
  x: number
  y: number
  width: number
  height: number
  scale: number
}

export interface FontFamilies {
  standard?: string
  fixed?: string
  serif?: string
  sansSerif?: string
  cursive?: string
  fantasy?: string
  math?: string
}

export interface ScriptFontFamilies {
  script: string
  fontFamilies: FontFamilies
}

export interface FontSizes {
  standard?: number
  fixed?: number
}

export type ClientNavigationReason =
  | 'anchorClick'
  | 'formSubmissionGet'
  | 'formSubmissionPost'
  | 'httpHeaderRefresh'
  | 'initialFrameNavigation'
  | 'metaTagRefresh'
  | 'other'
  | 'pageBlockInterstitial'
  | 'reload'
  | 'scriptInitiated'

export type ClientNavigationDisposition =
  | 'currentTab'
  | 'newTab'
  | 'newWindow'
  | 'download'

export interface InstallabilityErrorArgument {
  name: string
  value: string
}

export interface InstallabilityError {
  errorId: string
  errorArguments: InstallabilityErrorArgument[]
}

export type ReferrerPolicy =
  | 'noReferrer'
  | 'noReferrerWhenDowngrade'
  | 'origin'
  | 'originWhenCrossOrigin'
  | 'sameOrigin'
  | 'strictOrigin'
  | 'strictOriginWhenCrossOrigin'
  | 'unsafeUrl'

export interface CompilationCacheParams {
  url: string
  eager?: boolean
}

export interface FileFilter {
  name?: string
  accepts?: string[]
}

export interface FileHandler {
  action: string
  name: string
  icons?: ImageResource[]
  accepts?: FileFilter[]
  launchType: string
}

export interface ImageResource {
  url: string
  sizes?: string
  type?: string
}

export interface LaunchHandler {
  clientMode: string
}

export interface ProtocolHandler {
  protocol: string
  url: string
}

export interface RelatedApplication {
  id?: string
  url: string
}

export interface ScopeExtension {
  origin: string
  hasOriginWildcard: boolean
}

export interface Screenshot {
  image: ImageResource
  formFactor: string
  label?: string
}

export interface ShareTarget {
  action: string
  method: string
  enctype: string
  title?: string
  text?: string
  url?: string
  files?: FileFilter[]
}

export interface Shortcut {
  name: string
  url: string
}

export interface WebAppManifest {
  backgroundColor?: string
  description?: string
  dir?: string
  display?: string
  displayOverrides?: string[]
  fileHandlers?: FileHandler[]
  icons?: ImageResource[]
  id?: string
  lang?: string
  launchHandler?: LaunchHandler
  name?: string
  orientation?: string
  preferRelatedApplications?: boolean
  protocolHandlers?: ProtocolHandler[]
  relatedApplications?: RelatedApplication[]
  scope?: string
  scopeExtensions?: ScopeExtension[]
  screenshots?: Screenshot[]
  shareTarget?: ShareTarget
  shortName?: string
  shortcuts?: Shortcut[]
  startUrl?: string
  themeColor?: string
}

export type NavigationType = 'Navigation' | 'BackForwardCacheRestore'

export type BackForwardCacheNotRestoredReason =
  | 'NotPrimaryMainFrame'
  | 'BackForwardCacheDisabled'
  | 'RelatedActiveContentsExist'
  | 'HTTPStatusNotOK'
  | 'SchemeNotHTTPOrHTTPS'
  | 'Loading'
  | 'WasGrantedMediaAccess'
  | 'DisableForRenderFrameHostCalled'
  | 'DomainNotAllowed'
  | 'HTTPMethodNotGET'
  | 'SubframeIsNavigating'
  | 'Timeout'
  | 'CacheLimit'
  | 'JavaScriptExecution'
  | 'RendererProcessKilled'
  | 'RendererProcessCrashed'
  | 'SchedulerTrackedFeatureUsed'
  | 'ConflictingBrowsingInstance'
  | 'CacheFlushed'
  | 'ServiceWorkerVersionActivation'
  | 'SessionRestored'
  | 'ServiceWorkerPostMessage'
  | 'EnteredBackForwardCacheBeforeServiceWorkerHostAdded'
  | 'RenderFrameHostReused_SameSite'
  | 'RenderFrameHostReused_CrossSite'
  | 'ServiceWorkerClaim'
  | 'IgnoreEventAndEvict'
  | 'HaveInnerContents'
  | 'TimeoutPuttingInCache'
  | 'BackForwardCacheDisabledByLowMemory'
  | 'BackForwardCacheDisabledByCommandLine'
  | 'NetworkRequestDatapipeDrainedAsBytesConsumer'
  | 'NetworkRequestRedirected'
  | 'NetworkRequestTimeout'
  | 'NetworkExceedsBufferLimit'
  | 'NavigationCancelledWhileRestoring'
  | 'NotMostRecentNavigationEntry'
  | 'BackForwardCacheDisabledForPrerender'
  | 'UserAgentOverrideDiffers'
  | 'ForegroundCacheLimit'
  | 'ForwardCacheDisabled'
  | 'BrowsingInstanceNotSwapped'
  | 'BackForwardCacheDisabledForDelegate'
  | 'UnloadHandlerExistsInMainFrame'
  | 'UnloadHandlerExistsInSubFrame'
  | 'ServiceWorkerUnregistration'
  | 'CacheControlNoStore'
  | 'CacheControlNoStoreCookieModified'
  | 'CacheControlNoStoreHTTPOnlyCookieModified'
  | 'NoResponseHead'
  | 'Unknown'
  | 'ActivationNavigationsDisallowedForBug1234857'
  | 'ErrorDocument'
  | 'FencedFramesEmbedder'
  | 'CookieDisabled'
  | 'HTTPAuthRequired'
  | 'CookieFlushed'
  | 'BroadcastChannelOnMessage'
  | 'WebViewSettingsChanged'
  | 'WebViewJavaScriptObjectChanged'
  | 'WebViewMessageListenerInjected'
  | 'WebViewSafeBrowsingAllowlistChanged'
  | 'WebViewDocumentStartJavascriptChanged'
  | 'WebSocket'
  | 'WebTransport'
  | 'WebRTC'
  | 'MainResourceHasCacheControlNoStore'
  | 'MainResourceHasCacheControlNoCache'
  | 'SubresourceHasCacheControlNoStore'
  | 'SubresourceHasCacheControlNoCache'
  | 'ContainsPlugins'
  | 'DocumentLoaded'
  | 'OutstandingNetworkRequestOthers'
  | 'RequestedMIDIPermission'
  | 'RequestedAudioCapturePermission'
  | 'RequestedVideoCapturePermission'
  | 'RequestedBackForwardCacheBlockedSensors'
  | 'RequestedBackgroundWorkPermission'
  | 'BroadcastChannel'
  | 'WebXR'
  | 'SharedWorker'
  | 'SharedWorkerMessage'
  | 'SharedWorkerWithNoActiveClient'
  | 'WebLocks'
  | 'WebLocksContention'
  | 'WebHID'
  | 'WebBluetooth'
  | 'WebShare'
  | 'RequestedStorageAccessGrant'
  | 'WebNfc'
  | 'OutstandingNetworkRequestFetch'
  | 'OutstandingNetworkRequestXHR'
  | 'AppBanner'
  | 'Printing'
  | 'WebDatabase'
  | 'PictureInPicture'
  | 'SpeechRecognizer'
  | 'IdleManager'
  | 'PaymentManager'
  | 'SpeechSynthesis'
  | 'KeyboardLock'
  | 'WebOTPService'
  | 'OutstandingNetworkRequestDirectSocket'
  | 'InjectedJavascript'
  | 'InjectedStyleSheet'
  | 'KeepaliveRequest'
  | 'IndexedDBEvent'
  | 'Dummy'
  | 'JsNetworkRequestReceivedCacheControlNoStoreResource'
  | 'WebRTCUsedWithCCNS'
  | 'WebTransportUsedWithCCNS'
  | 'WebSocketUsedWithCCNS'
  | 'SmartCard'
  | 'LiveMediaStreamTrack'
  | 'UnloadHandler'
  | 'ParserAborted'
  | 'ContentSecurityHandler'
  | 'ContentWebAuthenticationAPI'
  | 'ContentFileChooser'
  | 'ContentSerial'
  | 'ContentFileSystemAccess'
  | 'ContentMediaDevicesDispatcherHost'
  | 'ContentWebBluetooth'
  | 'ContentWebUSB'
  | 'ContentMediaSessionService'
  | 'ContentScreenReader'
  | 'ContentDiscarded'
  | 'EmbedderPopupBlockerTabHelper'
  | 'EmbedderSafeBrowsingTriggeredPopupBlocker'
  | 'EmbedderSafeBrowsingThreatDetails'
  | 'EmbedderAppBannerManager'
  | 'EmbedderDomDistillerViewerSource'
  | 'EmbedderDomDistillerSelfDeletingRequestDelegate'
  | 'EmbedderOomInterventionTabHelper'
  | 'EmbedderOfflinePage'
  | 'EmbedderChromePasswordManagerClientBindCredentialManager'
  | 'EmbedderPermissionRequestManager'
  | 'EmbedderModalDialog'
  | 'EmbedderExtensions'
  | 'EmbedderExtensionMessaging'
  | 'EmbedderExtensionMessagingForOpenPort'
  | 'EmbedderExtensionSentMessageToCachedFrame'
  | 'RequestedByWebViewClient'
  | 'PostMessageByWebViewClient'
  | 'CacheControlNoStoreDeviceBoundSessionTerminated'
  | 'CacheLimitPrunedOnModerateMemoryPressure'
  | 'CacheLimitPrunedOnCriticalMemoryPressure'

export type BackForwardCacheNotRestoredReasonType =
  | 'SupportPending'
  | 'PageSupportNeeded'
  | 'Circumstantial'

export interface BackForwardCacheBlockingDetails {
  url?: string
  function?: string
  lineNumber: number
  columnNumber: number
}

export interface BackForwardCacheNotRestoredExplanation {
  type: BackForwardCacheNotRestoredReasonType
  reason: BackForwardCacheNotRestoredReason
  context?: string
  details?: BackForwardCacheBlockingDetails[]
}

export interface BackForwardCacheNotRestoredExplanationTree {
  url: string
  explanations: BackForwardCacheNotRestoredExplanation[]
  children: BackForwardCacheNotRestoredExplanationTree[]
}

// ══ Commands ══

export interface AddScriptToEvaluateOnLoadParams {
  scriptSource: string
}

export interface AddScriptToEvaluateOnLoadResult {
  identifier: ScriptIdentifier
}

export interface AddScriptToEvaluateOnNewDocumentParams {
  source: string
  worldName?: string
  includeCommandLineAPI?: boolean
  runImmediately?: boolean
}

export interface AddScriptToEvaluateOnNewDocumentResult {
  identifier: ScriptIdentifier
}

export interface CaptureScreenshotParams {
  format?: 'jpeg' | 'png' | 'webp'
  quality?: number
  clip?: Viewport
  fromSurface?: boolean
  captureBeyondViewport?: boolean
  optimizeForSpeed?: boolean
}

export interface CaptureScreenshotResult {
  data: string
}

export interface CaptureSnapshotParams {
  format?: 'mhtml'
}

export interface CaptureSnapshotResult {
  data: string
}

export interface CreateIsolatedWorldParams {
  frameId: FrameId
  worldName?: string
  grantUniveralAccess?: boolean
}

export interface CreateIsolatedWorldResult {
  executionContextId: ExecutionContextId
}

export interface DeleteCookieParams {
  cookieName: string
  url: string
}

export interface EnableParams {
  enableFileChooserOpenedEvent?: boolean
}

export interface GetAppManifestParams {
  manifestId?: string
}

export interface GetAppManifestResult {
  url: string
  errors: AppManifestError[]
  data?: string
  parsed?: AppManifestParsedProperties
  manifest: WebAppManifest
}

export interface GetInstallabilityErrorsResult {
  installabilityErrors: InstallabilityError[]
}

export interface GetManifestIconsResult {
  primaryIcon?: string
}

export interface GetAppIdResult {
  appId?: string
  recommendedId?: string
}

export interface GetAdScriptAncestryParams {
  frameId: FrameId
}

export interface GetAdScriptAncestryResult {
  adScriptAncestry?: AdAncestry
}

export interface GetFrameTreeResult {
  frameTree: FrameTree
}

export interface GetLayoutMetricsResult {
  layoutViewport: LayoutViewport
  visualViewport: VisualViewport
  contentSize: Rect
  cssLayoutViewport: LayoutViewport
  cssVisualViewport: VisualViewport
  cssContentSize: Rect
}

export interface GetNavigationHistoryResult {
  currentIndex: number
  entries: NavigationEntry[]
}

export interface GetResourceContentParams {
  frameId: FrameId
  url: string
}

export interface GetResourceContentResult {
  content: string
  base64Encoded: boolean
}

export interface GetResourceTreeResult {
  frameTree: FrameResourceTree
}

export interface HandleJavaScriptDialogParams {
  accept: boolean
  promptText?: string
}

export interface NavigateParams {
  url: string
  referrer?: string
  transitionType?: TransitionType
  frameId?: FrameId
  referrerPolicy?: ReferrerPolicy
}

export interface NavigateResult {
  frameId: FrameId
  loaderId?: LoaderId
  errorText?: string
  isDownload?: boolean
}

export interface NavigateToHistoryEntryParams {
  entryId: number
}

export interface PrintToPDFParams {
  landscape?: boolean
  displayHeaderFooter?: boolean
  printBackground?: boolean
  scale?: number
  paperWidth?: number
  paperHeight?: number
  marginTop?: number
  marginBottom?: number
  marginLeft?: number
  marginRight?: number
  pageRanges?: string
  headerTemplate?: string
  footerTemplate?: string
  preferCSSPageSize?: boolean
  transferMode?: 'ReturnAsBase64' | 'ReturnAsStream'
  generateTaggedPDF?: boolean
  generateDocumentOutline?: boolean
}

export interface PrintToPDFResult {
  data: string
  stream?: StreamHandle
}

export interface ReloadParams {
  ignoreCache?: boolean
  scriptToEvaluateOnLoad?: string
  loaderId?: LoaderId
}

export interface RemoveScriptToEvaluateOnLoadParams {
  identifier: ScriptIdentifier
}

export interface RemoveScriptToEvaluateOnNewDocumentParams {
  identifier: ScriptIdentifier
}

export interface ScreencastFrameAckParams {
  sessionId: number
}

export interface SearchInResourceParams {
  frameId: FrameId
  url: string
  query: string
  caseSensitive?: boolean
  isRegex?: boolean
}

export interface SearchInResourceResult {
  result: SearchMatch[]
}

export interface SetAdBlockingEnabledParams {
  enabled: boolean
}

export interface SetBypassCSPParams {
  enabled: boolean
}

export interface GetPermissionsPolicyStateParams {
  frameId: FrameId
}

export interface GetPermissionsPolicyStateResult {
  states: PermissionsPolicyFeatureState[]
}

export interface GetOriginTrialsParams {
  frameId: FrameId
}

export interface GetOriginTrialsResult {
  originTrials: OriginTrial[]
}

export interface SetDeviceMetricsOverrideParams {
  width: number
  height: number
  deviceScaleFactor: number
  mobile: boolean
  scale?: number
  screenWidth?: number
  screenHeight?: number
  positionX?: number
  positionY?: number
  dontSetVisibleSize?: boolean
  screenOrientation?: ScreenOrientation
  viewport?: Viewport
}

export interface SetDeviceOrientationOverrideParams {
  alpha: number
  beta: number
  gamma: number
}

export interface SetFontFamiliesParams {
  fontFamilies: FontFamilies
  forScripts?: ScriptFontFamilies[]
}

export interface SetFontSizesParams {
  fontSizes: FontSizes
}

export interface SetDocumentContentParams {
  frameId: FrameId
  html: string
}

export interface SetDownloadBehaviorParams {
  behavior: 'deny' | 'allow' | 'default'
  downloadPath?: string
}

export interface SetGeolocationOverrideParams {
  latitude?: number
  longitude?: number
  accuracy?: number
}

export interface SetLifecycleEventsEnabledParams {
  enabled: boolean
}

export interface SetTouchEmulationEnabledParams {
  enabled: boolean
  configuration?: 'mobile' | 'desktop'
}

export interface StartScreencastParams {
  format?: 'jpeg' | 'png'
  quality?: number
  maxWidth?: number
  maxHeight?: number
  everyNthFrame?: number
}

export interface SetWebLifecycleStateParams {
  state: 'frozen' | 'active'
}

export interface ProduceCompilationCacheParams {
  scripts: CompilationCacheParams[]
}

export interface AddCompilationCacheParams {
  url: string
  data: string
}

export interface SetSPCTransactionModeParams {
  mode:
    | 'none'
    | 'autoAccept'
    | 'autoChooseToAuthAnotherWay'
    | 'autoReject'
    | 'autoOptOut'
}

export interface SetRPHRegistrationModeParams {
  mode: 'none' | 'autoAccept' | 'autoReject'
}

export interface GenerateTestReportParams {
  message: string
  group?: string
}

export interface SetInterceptFileChooserDialogParams {
  enabled: boolean
  cancel?: boolean
}

export interface SetPrerenderingAllowedParams {
  isAllowed: boolean
}

export interface GetAnnotatedPageContentParams {
  includeActionableInformation?: boolean
}

export interface GetAnnotatedPageContentResult {
  content: string
}

// ══ Events ══

export interface DomContentEventFiredEvent {
  timestamp: MonotonicTime
}

export interface FileChooserOpenedEvent {
  frameId: FrameId
  mode: 'selectSingle' | 'selectMultiple'
  backendNodeId?: BackendNodeId
}

export interface FrameAttachedEvent {
  frameId: FrameId
  parentFrameId: FrameId
  stack?: StackTrace
}

export interface FrameClearedScheduledNavigationEvent {
  frameId: FrameId
}

export interface FrameDetachedEvent {
  frameId: FrameId
  reason: 'remove' | 'swap'
}

export interface FrameSubtreeWillBeDetachedEvent {
  frameId: FrameId
}

export interface FrameNavigatedEvent {
  frame: Frame
  type: NavigationType
}

export interface DocumentOpenedEvent {
  frame: Frame
}

export interface FrameStartedNavigatingEvent {
  frameId: FrameId
  url: string
  loaderId: LoaderId
  navigationType:
    | 'reload'
    | 'reloadBypassingCache'
    | 'restore'
    | 'restoreWithPost'
    | 'historySameDocument'
    | 'historyDifferentDocument'
    | 'sameDocument'
    | 'differentDocument'
}

export interface FrameRequestedNavigationEvent {
  frameId: FrameId
  reason: ClientNavigationReason
  url: string
  disposition: ClientNavigationDisposition
}

export interface FrameScheduledNavigationEvent {
  frameId: FrameId
  delay: number
  reason: ClientNavigationReason
  url: string
}

export interface FrameStartedLoadingEvent {
  frameId: FrameId
}

export interface FrameStoppedLoadingEvent {
  frameId: FrameId
}

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
}

export interface JavascriptDialogClosedEvent {
  frameId: FrameId
  result: boolean
  userInput: string
}

export interface JavascriptDialogOpeningEvent {
  url: string
  frameId: FrameId
  message: string
  type: DialogType
  hasBrowserHandler: boolean
  defaultPrompt?: string
}

export interface LifecycleEventEvent {
  frameId: FrameId
  loaderId: LoaderId
  name: string
  timestamp: MonotonicTime
}

export interface BackForwardCacheNotUsedEvent {
  loaderId: LoaderId
  frameId: FrameId
  notRestoredExplanations: BackForwardCacheNotRestoredExplanation[]
  notRestoredExplanationsTree?: BackForwardCacheNotRestoredExplanationTree
}

export interface LoadEventFiredEvent {
  timestamp: MonotonicTime
}

export interface NavigatedWithinDocumentEvent {
  frameId: FrameId
  url: string
  navigationType: 'fragment' | 'historyApi' | 'other'
}

export interface ScreencastFrameEvent {
  data: string
  metadata: ScreencastFrameMetadata
  sessionId: number
}

export interface ScreencastVisibilityChangedEvent {
  visible: boolean
}

export interface WindowOpenEvent {
  url: string
  windowName: string
  windowFeatures: string[]
  userGesture: boolean
}

export interface CompilationCacheProducedEvent {
  url: string
  data: string
}
