// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { SearchMatch } from './debugger'
import type { UserAgentMetadata } from './emulation'
import type { StreamHandle } from './io'
import type { FrameId } from './page'
import type { ScriptId, StackTrace, UniqueDebuggerId } from './runtime'
import type { CertificateId, MixedContentType, SecurityState } from './security'

// ══ Types ══

export type ResourceType =
  | 'Document'
  | 'Stylesheet'
  | 'Image'
  | 'Media'
  | 'Font'
  | 'Script'
  | 'TextTrack'
  | 'XHR'
  | 'Fetch'
  | 'Prefetch'
  | 'EventSource'
  | 'WebSocket'
  | 'Manifest'
  | 'SignedExchange'
  | 'Ping'
  | 'CSPViolationReport'
  | 'Preflight'
  | 'FedCM'
  | 'Other'

export type LoaderId = string

export type RequestId = string

export type InterceptionId = string

export type ErrorReason =
  | 'Failed'
  | 'Aborted'
  | 'TimedOut'
  | 'AccessDenied'
  | 'ConnectionClosed'
  | 'ConnectionReset'
  | 'ConnectionRefused'
  | 'ConnectionAborted'
  | 'ConnectionFailed'
  | 'NameNotResolved'
  | 'InternetDisconnected'
  | 'AddressUnreachable'
  | 'BlockedByClient'
  | 'BlockedByResponse'

export type TimeSinceEpoch = number

export type MonotonicTime = number

export type Headers = Record<string, unknown>

export type ConnectionType =
  | 'none'
  | 'cellular2g'
  | 'cellular3g'
  | 'cellular4g'
  | 'bluetooth'
  | 'ethernet'
  | 'wifi'
  | 'wimax'
  | 'other'

export type CookieSameSite = 'Strict' | 'Lax' | 'None'

export type CookiePriority = 'Low' | 'Medium' | 'High'

export type CookieSourceScheme = 'Unset' | 'NonSecure' | 'Secure'

export interface ResourceTiming {
  requestTime: number
  proxyStart: number
  proxyEnd: number
  dnsStart: number
  dnsEnd: number
  connectStart: number
  connectEnd: number
  sslStart: number
  sslEnd: number
  workerStart: number
  workerReady: number
  workerFetchStart: number
  workerRespondWithSettled: number
  workerRouterEvaluationStart?: number
  workerCacheLookupStart?: number
  sendStart: number
  sendEnd: number
  pushStart: number
  pushEnd: number
  receiveHeadersStart: number
  receiveHeadersEnd: number
}

export type ResourcePriority =
  | 'VeryLow'
  | 'Low'
  | 'Medium'
  | 'High'
  | 'VeryHigh'

export type RenderBlockingBehavior =
  | 'Blocking'
  | 'InBodyParserBlocking'
  | 'NonBlocking'
  | 'NonBlockingDynamic'
  | 'PotentiallyBlocking'

export interface PostDataEntry {
  bytes?: string
}

export interface Request {
  url: string
  urlFragment?: string
  method: string
  headers: Headers
  postData?: string
  hasPostData?: boolean
  postDataEntries?: PostDataEntry[]
  mixedContentType?: MixedContentType
  initialPriority: ResourcePriority
  referrerPolicy:
    | 'unsafe-url'
    | 'no-referrer-when-downgrade'
    | 'no-referrer'
    | 'origin'
    | 'origin-when-cross-origin'
    | 'same-origin'
    | 'strict-origin'
    | 'strict-origin-when-cross-origin'
  isLinkPreload?: boolean
  trustTokenParams?: TrustTokenParams
  isSameSite?: boolean
  isAdRelated?: boolean
}

export interface SignedCertificateTimestamp {
  status: string
  origin: string
  logDescription: string
  logId: string
  timestamp: number
  hashAlgorithm: string
  signatureAlgorithm: string
  signatureData: string
}

export interface SecurityDetails {
  protocol: string
  keyExchange: string
  keyExchangeGroup?: string
  cipher: string
  mac?: string
  certificateId: CertificateId
  subjectName: string
  sanList: string[]
  issuer: string
  validFrom: TimeSinceEpoch
  validTo: TimeSinceEpoch
  signedCertificateTimestampList: SignedCertificateTimestamp[]
  certificateTransparencyCompliance: CertificateTransparencyCompliance
  serverSignatureAlgorithm?: number
  encryptedClientHello: boolean
}

export type CertificateTransparencyCompliance =
  | 'unknown'
  | 'not-compliant'
  | 'compliant'

export type BlockedReason =
  | 'other'
  | 'csp'
  | 'mixed-content'
  | 'origin'
  | 'inspector'
  | 'integrity'
  | 'subresource-filter'
  | 'content-type'
  | 'coep-frame-resource-needs-coep-header'
  | 'coop-sandboxed-iframe-cannot-navigate-to-coop-page'
  | 'corp-not-same-origin'
  | 'corp-not-same-origin-after-defaulted-to-same-origin-by-coep'
  | 'corp-not-same-origin-after-defaulted-to-same-origin-by-dip'
  | 'corp-not-same-origin-after-defaulted-to-same-origin-by-coep-and-dip'
  | 'corp-not-same-site'
  | 'sri-message-signature-mismatch'

export type CorsError =
  | 'DisallowedByMode'
  | 'InvalidResponse'
  | 'WildcardOriginNotAllowed'
  | 'MissingAllowOriginHeader'
  | 'MultipleAllowOriginValues'
  | 'InvalidAllowOriginValue'
  | 'AllowOriginMismatch'
  | 'InvalidAllowCredentials'
  | 'CorsDisabledScheme'
  | 'PreflightInvalidStatus'
  | 'PreflightDisallowedRedirect'
  | 'PreflightWildcardOriginNotAllowed'
  | 'PreflightMissingAllowOriginHeader'
  | 'PreflightMultipleAllowOriginValues'
  | 'PreflightInvalidAllowOriginValue'
  | 'PreflightAllowOriginMismatch'
  | 'PreflightInvalidAllowCredentials'
  | 'PreflightMissingAllowExternal'
  | 'PreflightInvalidAllowExternal'
  | 'InvalidAllowMethodsPreflightResponse'
  | 'InvalidAllowHeadersPreflightResponse'
  | 'MethodDisallowedByPreflightResponse'
  | 'HeaderDisallowedByPreflightResponse'
  | 'RedirectContainsCredentials'
  | 'InsecureLocalNetwork'
  | 'InvalidLocalNetworkAccess'
  | 'NoCorsRedirectModeNotFollow'
  | 'LocalNetworkAccessPermissionDenied'

export interface CorsErrorStatus {
  corsError: CorsError
  failedParameter: string
}

export type ServiceWorkerResponseSource =
  | 'cache-storage'
  | 'http-cache'
  | 'fallback-code'
  | 'network'

export interface TrustTokenParams {
  operation: TrustTokenOperationType
  refreshPolicy: 'UseCached' | 'Refresh'
  issuers?: string[]
}

export type TrustTokenOperationType = 'Issuance' | 'Redemption' | 'Signing'

export type AlternateProtocolUsage =
  | 'alternativeJobWonWithoutRace'
  | 'alternativeJobWonRace'
  | 'mainJobWonRace'
  | 'mappingMissing'
  | 'broken'
  | 'dnsAlpnH3JobWonWithoutRace'
  | 'dnsAlpnH3JobWonRace'
  | 'unspecifiedReason'

export type ServiceWorkerRouterSource =
  | 'network'
  | 'cache'
  | 'fetch-event'
  | 'race-network-and-fetch-handler'
  | 'race-network-and-cache'

export interface ServiceWorkerRouterInfo {
  ruleIdMatched?: number
  matchedSourceType?: ServiceWorkerRouterSource
  actualSourceType?: ServiceWorkerRouterSource
}

export interface Response {
  url: string
  status: number
  statusText: string
  headers: Headers
  headersText?: string
  mimeType: string
  charset: string
  requestHeaders?: Headers
  requestHeadersText?: string
  connectionReused: boolean
  connectionId: number
  remoteIPAddress?: string
  remotePort?: number
  fromDiskCache?: boolean
  fromServiceWorker?: boolean
  fromPrefetchCache?: boolean
  fromEarlyHints?: boolean
  serviceWorkerRouterInfo?: ServiceWorkerRouterInfo
  encodedDataLength: number
  timing?: ResourceTiming
  serviceWorkerResponseSource?: ServiceWorkerResponseSource
  responseTime?: TimeSinceEpoch
  cacheStorageCacheName?: string
  protocol?: string
  alternateProtocolUsage?: AlternateProtocolUsage
  securityState: SecurityState
  securityDetails?: SecurityDetails
}

export interface WebSocketRequest {
  headers: Headers
}

export interface WebSocketResponse {
  status: number
  statusText: string
  headers: Headers
  headersText?: string
  requestHeaders?: Headers
  requestHeadersText?: string
}

export interface WebSocketFrame {
  opcode: number
  mask: boolean
  payloadData: string
}

export interface CachedResource {
  url: string
  type: ResourceType
  response?: Response
  bodySize: number
}

export interface Initiator {
  type:
    | 'parser'
    | 'script'
    | 'preload'
    | 'SignedExchange'
    | 'preflight'
    | 'FedCM'
    | 'other'
  stack?: StackTrace
  url?: string
  lineNumber?: number
  columnNumber?: number
  requestId?: RequestId
}

export interface CookiePartitionKey {
  topLevelSite: string
  hasCrossSiteAncestor: boolean
}

export interface Cookie {
  name: string
  value: string
  domain: string
  path: string
  expires: number
  size: number
  httpOnly: boolean
  secure: boolean
  session: boolean
  sameSite?: CookieSameSite
  priority: CookiePriority
  sourceScheme: CookieSourceScheme
  sourcePort: number
  partitionKey?: CookiePartitionKey
  partitionKeyOpaque?: boolean
}

export type SetCookieBlockedReason =
  | 'SecureOnly'
  | 'SameSiteStrict'
  | 'SameSiteLax'
  | 'SameSiteUnspecifiedTreatedAsLax'
  | 'SameSiteNoneInsecure'
  | 'UserPreferences'
  | 'ThirdPartyPhaseout'
  | 'ThirdPartyBlockedInFirstPartySet'
  | 'SyntaxError'
  | 'SchemeNotSupported'
  | 'OverwriteSecure'
  | 'InvalidDomain'
  | 'InvalidPrefix'
  | 'UnknownError'
  | 'SchemefulSameSiteStrict'
  | 'SchemefulSameSiteLax'
  | 'SchemefulSameSiteUnspecifiedTreatedAsLax'
  | 'NameValuePairExceedsMaxSize'
  | 'DisallowedCharacter'
  | 'NoCookieContent'

export type CookieBlockedReason =
  | 'SecureOnly'
  | 'NotOnPath'
  | 'DomainMismatch'
  | 'SameSiteStrict'
  | 'SameSiteLax'
  | 'SameSiteUnspecifiedTreatedAsLax'
  | 'SameSiteNoneInsecure'
  | 'UserPreferences'
  | 'ThirdPartyPhaseout'
  | 'ThirdPartyBlockedInFirstPartySet'
  | 'UnknownError'
  | 'SchemefulSameSiteStrict'
  | 'SchemefulSameSiteLax'
  | 'SchemefulSameSiteUnspecifiedTreatedAsLax'
  | 'NameValuePairExceedsMaxSize'
  | 'PortMismatch'
  | 'SchemeMismatch'
  | 'AnonymousContext'

export type CookieExemptionReason =
  | 'None'
  | 'UserSetting'
  | 'TPCDMetadata'
  | 'TPCDDeprecationTrial'
  | 'TopLevelTPCDDeprecationTrial'
  | 'TPCDHeuristics'
  | 'EnterprisePolicy'
  | 'StorageAccess'
  | 'TopLevelStorageAccess'
  | 'Scheme'
  | 'SameSiteNoneCookiesInSandbox'

export interface BlockedSetCookieWithReason {
  blockedReasons: SetCookieBlockedReason[]
  cookieLine: string
  cookie?: Cookie
}

export interface ExemptedSetCookieWithReason {
  exemptionReason: CookieExemptionReason
  cookieLine: string
  cookie: Cookie
}

export interface AssociatedCookie {
  cookie: Cookie
  blockedReasons: CookieBlockedReason[]
  exemptionReason?: CookieExemptionReason
}

export interface CookieParam {
  name: string
  value: string
  url?: string
  domain?: string
  path?: string
  secure?: boolean
  httpOnly?: boolean
  sameSite?: CookieSameSite
  expires?: TimeSinceEpoch
  priority?: CookiePriority
  sourceScheme?: CookieSourceScheme
  sourcePort?: number
  partitionKey?: CookiePartitionKey
}

export interface AuthChallenge {
  source?: 'Server' | 'Proxy'
  origin: string
  scheme: string
  realm: string
}

export interface AuthChallengeResponse {
  response: 'Default' | 'CancelAuth' | 'ProvideCredentials'
  username?: string
  password?: string
}

export type InterceptionStage = 'Request' | 'HeadersReceived'

export interface RequestPattern {
  urlPattern?: string
  resourceType?: ResourceType
  interceptionStage?: InterceptionStage
}

export interface SignedExchangeSignature {
  label: string
  signature: string
  integrity: string
  certUrl?: string
  certSha256?: string
  validityUrl: string
  date: number
  expires: number
  certificates?: string[]
}

export interface SignedExchangeHeader {
  requestUrl: string
  responseCode: number
  responseHeaders: Headers
  signatures: SignedExchangeSignature[]
  headerIntegrity: string
}

export type SignedExchangeErrorField =
  | 'signatureSig'
  | 'signatureIntegrity'
  | 'signatureCertUrl'
  | 'signatureCertSha256'
  | 'signatureValidityUrl'
  | 'signatureTimestamps'

export interface SignedExchangeError {
  message: string
  signatureIndex?: number
  errorField?: SignedExchangeErrorField
}

export interface SignedExchangeInfo {
  outerResponse: Response
  hasExtraInfo: boolean
  header?: SignedExchangeHeader
  securityDetails?: SecurityDetails
  errors?: SignedExchangeError[]
}

export type ContentEncoding = 'deflate' | 'gzip' | 'br' | 'zstd'

export interface NetworkConditions {
  urlPattern: string
  latency: number
  downloadThroughput: number
  uploadThroughput: number
  connectionType?: ConnectionType
  packetLoss?: number
  packetQueueLength?: number
  packetReordering?: boolean
}

export interface BlockPattern {
  urlPattern: string
  block: boolean
}

export type DirectSocketDnsQueryType = 'ipv4' | 'ipv6'

export interface DirectTCPSocketOptions {
  noDelay: boolean
  keepAliveDelay?: number
  sendBufferSize?: number
  receiveBufferSize?: number
  dnsQueryType?: DirectSocketDnsQueryType
}

export interface DirectUDPSocketOptions {
  remoteAddr?: string
  remotePort?: number
  localAddr?: string
  localPort?: number
  dnsQueryType?: DirectSocketDnsQueryType
  sendBufferSize?: number
  receiveBufferSize?: number
  multicastLoopback?: boolean
  multicastTimeToLive?: number
  multicastAllowAddressSharing?: boolean
}

export interface DirectUDPMessage {
  data: string
  remoteAddr?: string
  remotePort?: number
}

export type LocalNetworkAccessRequestPolicy =
  | 'Allow'
  | 'BlockFromInsecureToMorePrivate'
  | 'WarnFromInsecureToMorePrivate'
  | 'PermissionBlock'
  | 'PermissionWarn'

export type IPAddressSpace = 'Loopback' | 'Local' | 'Public' | 'Unknown'

export interface ConnectTiming {
  requestTime: number
}

export interface ClientSecurityState {
  initiatorIsSecureContext: boolean
  initiatorIPAddressSpace: IPAddressSpace
  localNetworkAccessRequestPolicy: LocalNetworkAccessRequestPolicy
}

export interface AdScriptIdentifier {
  scriptId: ScriptId
  debuggerId: UniqueDebuggerId
  name: string
}

export interface AdAncestry {
  ancestryChain: AdScriptIdentifier[]
  rootScriptFilterlistRule?: string
}

export interface AdProvenance {
  filterlistRule?: string
  adScriptAncestry?: AdAncestry
}

export type CrossOriginOpenerPolicyValue =
  | 'SameOrigin'
  | 'SameOriginAllowPopups'
  | 'RestrictProperties'
  | 'UnsafeNone'
  | 'SameOriginPlusCoep'
  | 'RestrictPropertiesPlusCoep'
  | 'NoopenerAllowPopups'

export interface CrossOriginOpenerPolicyStatus {
  value: CrossOriginOpenerPolicyValue
  reportOnlyValue: CrossOriginOpenerPolicyValue
  reportingEndpoint?: string
  reportOnlyReportingEndpoint?: string
}

export type CrossOriginEmbedderPolicyValue =
  | 'None'
  | 'Credentialless'
  | 'RequireCorp'

export interface CrossOriginEmbedderPolicyStatus {
  value: CrossOriginEmbedderPolicyValue
  reportOnlyValue: CrossOriginEmbedderPolicyValue
  reportingEndpoint?: string
  reportOnlyReportingEndpoint?: string
}

export type ContentSecurityPolicySource = 'HTTP' | 'Meta'

export interface ContentSecurityPolicyStatus {
  effectiveDirectives: string
  isEnforced: boolean
  source: ContentSecurityPolicySource
}

export interface SecurityIsolationStatus {
  coop?: CrossOriginOpenerPolicyStatus
  coep?: CrossOriginEmbedderPolicyStatus
  csp?: ContentSecurityPolicyStatus[]
}

export type ReportStatus = 'Queued' | 'Pending' | 'MarkedForRemoval' | 'Success'

export type ReportId = string

export interface ReportingApiReport {
  id: ReportId
  initiatorUrl: string
  destination: string
  type: string
  timestamp: TimeSinceEpoch
  depth: number
  completedAttempts: number
  body: Record<string, unknown>
  status: ReportStatus
}

export interface ReportingApiEndpoint {
  url: string
  groupName: string
}

export interface DeviceBoundSessionKey {
  site: string
  id: string
}

export interface DeviceBoundSessionWithUsage {
  sessionKey: DeviceBoundSessionKey
  usage:
    | 'NotInScope'
    | 'InScopeRefreshNotYetNeeded'
    | 'InScopeRefreshNotAllowed'
    | 'ProactiveRefreshNotPossible'
    | 'ProactiveRefreshAttempted'
    | 'Deferred'
}

export interface DeviceBoundSessionCookieCraving {
  name: string
  domain: string
  path: string
  secure: boolean
  httpOnly: boolean
  sameSite?: CookieSameSite
}

export interface DeviceBoundSessionUrlRule {
  ruleType: 'Exclude' | 'Include'
  hostPattern: string
  pathPrefix: string
}

export interface DeviceBoundSessionInclusionRules {
  origin: string
  includeSite: boolean
  urlRules: DeviceBoundSessionUrlRule[]
}

export interface DeviceBoundSession {
  key: DeviceBoundSessionKey
  refreshUrl: string
  inclusionRules: DeviceBoundSessionInclusionRules
  cookieCravings: DeviceBoundSessionCookieCraving[]
  expiryDate: TimeSinceEpoch
  cachedChallenge?: string
  allowedRefreshInitiators: string[]
}

export type DeviceBoundSessionEventId = string

export type DeviceBoundSessionFetchResult =
  | 'Success'
  | 'KeyError'
  | 'SigningError'
  | 'ServerRequestedTermination'
  | 'InvalidSessionId'
  | 'InvalidChallenge'
  | 'TooManyChallenges'
  | 'InvalidFetcherUrl'
  | 'InvalidRefreshUrl'
  | 'TransientHttpError'
  | 'ScopeOriginSameSiteMismatch'
  | 'RefreshUrlSameSiteMismatch'
  | 'MismatchedSessionId'
  | 'MissingScope'
  | 'NoCredentials'
  | 'SubdomainRegistrationWellKnownUnavailable'
  | 'SubdomainRegistrationUnauthorized'
  | 'SubdomainRegistrationWellKnownMalformed'
  | 'SessionProviderWellKnownUnavailable'
  | 'RelyingPartyWellKnownUnavailable'
  | 'FederatedKeyThumbprintMismatch'
  | 'InvalidFederatedSessionUrl'
  | 'InvalidFederatedKey'
  | 'TooManyRelyingOriginLabels'
  | 'BoundCookieSetForbidden'
  | 'NetError'
  | 'ProxyError'
  | 'EmptySessionConfig'
  | 'InvalidCredentialsConfig'
  | 'InvalidCredentialsType'
  | 'InvalidCredentialsEmptyName'
  | 'InvalidCredentialsCookie'
  | 'PersistentHttpError'
  | 'RegistrationAttemptedChallenge'
  | 'InvalidScopeOrigin'
  | 'ScopeOriginContainsPath'
  | 'RefreshInitiatorNotString'
  | 'RefreshInitiatorInvalidHostPattern'
  | 'InvalidScopeSpecification'
  | 'MissingScopeSpecificationType'
  | 'EmptyScopeSpecificationDomain'
  | 'EmptyScopeSpecificationPath'
  | 'InvalidScopeSpecificationType'
  | 'InvalidScopeIncludeSite'
  | 'MissingScopeIncludeSite'
  | 'FederatedNotAuthorizedByProvider'
  | 'FederatedNotAuthorizedByRelyingParty'
  | 'SessionProviderWellKnownMalformed'
  | 'SessionProviderWellKnownHasProviderOrigin'
  | 'RelyingPartyWellKnownMalformed'
  | 'RelyingPartyWellKnownHasRelyingOrigins'
  | 'InvalidFederatedSessionProviderSessionMissing'
  | 'InvalidFederatedSessionWrongProviderOrigin'
  | 'InvalidCredentialsCookieCreationTime'
  | 'InvalidCredentialsCookieName'
  | 'InvalidCredentialsCookieParsing'
  | 'InvalidCredentialsCookieUnpermittedAttribute'
  | 'InvalidCredentialsCookieInvalidDomain'
  | 'InvalidCredentialsCookiePrefix'
  | 'InvalidScopeRulePath'
  | 'InvalidScopeRuleHostPattern'
  | 'ScopeRuleOriginScopedHostPatternMismatch'
  | 'ScopeRuleSiteScopedHostPatternMismatch'
  | 'SigningQuotaExceeded'
  | 'InvalidConfigJson'
  | 'InvalidFederatedSessionProviderFailedToRestoreKey'
  | 'FailedToUnwrapKey'
  | 'SessionDeletedDuringRefresh'

export interface DeviceBoundSessionFailedRequest {
  requestUrl: string
  netError?: string
  responseError?: number
  responseErrorBody?: string
}

export interface CreationEventDetails {
  fetchResult: DeviceBoundSessionFetchResult
  newSession?: DeviceBoundSession
  failedRequest?: DeviceBoundSessionFailedRequest
}

export interface RefreshEventDetails {
  refreshResult:
    | 'Refreshed'
    | 'InitializedService'
    | 'Unreachable'
    | 'ServerError'
    | 'RefreshQuotaExceeded'
    | 'FatalError'
    | 'SigningQuotaExceeded'
  fetchResult?: DeviceBoundSessionFetchResult
  newSession?: DeviceBoundSession
  wasFullyProactiveRefresh: boolean
  failedRequest?: DeviceBoundSessionFailedRequest
}

export interface TerminationEventDetails {
  deletionReason:
    | 'Expired'
    | 'FailedToRestoreKey'
    | 'FailedToUnwrapKey'
    | 'StoragePartitionCleared'
    | 'ClearBrowsingData'
    | 'ServerRequested'
    | 'InvalidSessionParams'
    | 'RefreshFatalError'
}

export interface ChallengeEventDetails {
  challengeResult:
    | 'Success'
    | 'NoSessionId'
    | 'NoSessionMatch'
    | 'CantSetBoundCookie'
  challenge: string
}

export interface LoadNetworkResourcePageResult {
  success: boolean
  netError?: number
  netErrorName?: string
  httpStatusCode?: number
  stream?: StreamHandle
  headers?: Headers
}

export interface LoadNetworkResourceOptions {
  disableCache: boolean
  includeCredentials: boolean
}

// ══ Commands ══

export interface SetAcceptedEncodingsParams {
  encodings: ContentEncoding[]
}

export interface CanClearBrowserCacheResult {
  result: boolean
}

export interface CanClearBrowserCookiesResult {
  result: boolean
}

export interface CanEmulateNetworkConditionsResult {
  result: boolean
}

export interface ContinueInterceptedRequestParams {
  interceptionId: InterceptionId
  errorReason?: ErrorReason
  rawResponse?: string
  url?: string
  method?: string
  postData?: string
  headers?: Headers
  authChallengeResponse?: AuthChallengeResponse
}

export interface DeleteCookiesParams {
  name: string
  url?: string
  domain?: string
  path?: string
  partitionKey?: CookiePartitionKey
}

export interface EmulateNetworkConditionsParams {
  offline: boolean
  latency: number
  downloadThroughput: number
  uploadThroughput: number
  connectionType?: ConnectionType
  packetLoss?: number
  packetQueueLength?: number
  packetReordering?: boolean
}

export interface EmulateNetworkConditionsByRuleParams {
  offline: boolean
  matchedNetworkConditions: NetworkConditions[]
}

export interface EmulateNetworkConditionsByRuleResult {
  ruleIds: string[]
}

export interface OverrideNetworkStateParams {
  offline: boolean
  latency: number
  downloadThroughput: number
  uploadThroughput: number
  connectionType?: ConnectionType
}

export interface EnableParams {
  maxTotalBufferSize?: number
  maxResourceBufferSize?: number
  maxPostDataSize?: number
  reportDirectSocketTraffic?: boolean
  enableDurableMessages?: boolean
}

export interface ConfigureDurableMessagesParams {
  maxTotalBufferSize?: number
  maxResourceBufferSize?: number
}

export interface GetAllCookiesResult {
  cookies: Cookie[]
}

export interface GetCertificateParams {
  origin: string
}

export interface GetCertificateResult {
  tableNames: string[]
}

export interface GetCookiesParams {
  urls?: string[]
}

export interface GetCookiesResult {
  cookies: Cookie[]
}

export interface GetResponseBodyParams {
  requestId: RequestId
}

export interface GetResponseBodyResult {
  body: string
  base64Encoded: boolean
}

export interface GetRequestPostDataParams {
  requestId: RequestId
}

export interface GetRequestPostDataResult {
  postData: string
  base64Encoded: boolean
}

export interface GetResponseBodyForInterceptionParams {
  interceptionId: InterceptionId
}

export interface GetResponseBodyForInterceptionResult {
  body: string
  base64Encoded: boolean
}

export interface TakeResponseBodyForInterceptionAsStreamParams {
  interceptionId: InterceptionId
}

export interface TakeResponseBodyForInterceptionAsStreamResult {
  stream: StreamHandle
}

export interface ReplayXHRParams {
  requestId: RequestId
}

export interface SearchInResponseBodyParams {
  requestId: RequestId
  query: string
  caseSensitive?: boolean
  isRegex?: boolean
}

export interface SearchInResponseBodyResult {
  result: SearchMatch[]
}

export interface SetBlockedURLsParams {
  urlPatterns?: BlockPattern[]
  urls?: string[]
}

export interface SetBypassServiceWorkerParams {
  bypass: boolean
}

export interface SetCacheDisabledParams {
  cacheDisabled: boolean
}

export interface SetCookieParams {
  name: string
  value: string
  url?: string
  domain?: string
  path?: string
  secure?: boolean
  httpOnly?: boolean
  sameSite?: CookieSameSite
  expires?: TimeSinceEpoch
  priority?: CookiePriority
  sourceScheme?: CookieSourceScheme
  sourcePort?: number
  partitionKey?: CookiePartitionKey
}

export interface SetCookieResult {
  success: boolean
}

export interface SetCookiesParams {
  cookies: CookieParam[]
}

export interface SetExtraHTTPHeadersParams {
  headers: Headers
}

export interface SetAttachDebugStackParams {
  enabled: boolean
}

export interface SetRequestInterceptionParams {
  patterns: RequestPattern[]
}

export interface SetUserAgentOverrideParams {
  userAgent: string
  acceptLanguage?: string
  platform?: string
  userAgentMetadata?: UserAgentMetadata
}

export interface StreamResourceContentParams {
  requestId: RequestId
}

export interface StreamResourceContentResult {
  bufferedData: string
}

export interface GetSecurityIsolationStatusParams {
  frameId?: FrameId
}

export interface GetSecurityIsolationStatusResult {
  status: SecurityIsolationStatus
}

export interface EnableReportingApiParams {
  enable: boolean
}

export interface EnableDeviceBoundSessionsParams {
  enable: boolean
}

export interface FetchSchemefulSiteParams {
  origin: string
}

export interface FetchSchemefulSiteResult {
  schemefulSite: string
}

export interface LoadNetworkResourceParams {
  frameId?: FrameId
  url: string
  options: LoadNetworkResourceOptions
}

export interface LoadNetworkResourceResult {
  resource: LoadNetworkResourcePageResult
}

export interface SetCookieControlsParams {
  enableThirdPartyCookieRestriction: boolean
  disableThirdPartyCookieMetadata: boolean
  disableThirdPartyCookieHeuristics: boolean
}

// ══ Events ══

export interface DataReceivedEvent {
  requestId: RequestId
  timestamp: MonotonicTime
  dataLength: number
  encodedDataLength: number
  data?: string
}

export interface EventSourceMessageReceivedEvent {
  requestId: RequestId
  timestamp: MonotonicTime
  eventName: string
  eventId: string
  data: string
}

export interface LoadingFailedEvent {
  requestId: RequestId
  timestamp: MonotonicTime
  type: ResourceType
  errorText: string
  canceled?: boolean
  blockedReason?: BlockedReason
  corsErrorStatus?: CorsErrorStatus
}

export interface LoadingFinishedEvent {
  requestId: RequestId
  timestamp: MonotonicTime
  encodedDataLength: number
}

export interface RequestInterceptedEvent {
  interceptionId: InterceptionId
  request: Request
  frameId: FrameId
  resourceType: ResourceType
  isNavigationRequest: boolean
  isDownload?: boolean
  redirectUrl?: string
  authChallenge?: AuthChallenge
  responseErrorReason?: ErrorReason
  responseStatusCode?: number
  responseHeaders?: Headers
  requestId?: RequestId
}

export interface RequestServedFromCacheEvent {
  requestId: RequestId
}

export interface RequestWillBeSentEvent {
  requestId: RequestId
  loaderId: LoaderId
  documentURL: string
  request: Request
  timestamp: MonotonicTime
  wallTime: TimeSinceEpoch
  initiator: Initiator
  redirectHasExtraInfo: boolean
  redirectResponse?: Response
  type?: ResourceType
  frameId?: FrameId
  hasUserGesture?: boolean
  renderBlockingBehavior?: RenderBlockingBehavior
}

export interface ResourceChangedPriorityEvent {
  requestId: RequestId
  newPriority: ResourcePriority
  timestamp: MonotonicTime
}

export interface SignedExchangeReceivedEvent {
  requestId: RequestId
  info: SignedExchangeInfo
}

export interface ResponseReceivedEvent {
  requestId: RequestId
  loaderId: LoaderId
  timestamp: MonotonicTime
  type: ResourceType
  response: Response
  hasExtraInfo: boolean
  frameId?: FrameId
}

export interface WebSocketClosedEvent {
  requestId: RequestId
  timestamp: MonotonicTime
}

export interface WebSocketCreatedEvent {
  requestId: RequestId
  url: string
  initiator?: Initiator
}

export interface WebSocketFrameErrorEvent {
  requestId: RequestId
  timestamp: MonotonicTime
  errorMessage: string
}

export interface WebSocketFrameReceivedEvent {
  requestId: RequestId
  timestamp: MonotonicTime
  response: WebSocketFrame
}

export interface WebSocketFrameSentEvent {
  requestId: RequestId
  timestamp: MonotonicTime
  response: WebSocketFrame
}

export interface WebSocketHandshakeResponseReceivedEvent {
  requestId: RequestId
  timestamp: MonotonicTime
  response: WebSocketResponse
}

export interface WebSocketWillSendHandshakeRequestEvent {
  requestId: RequestId
  timestamp: MonotonicTime
  wallTime: TimeSinceEpoch
  request: WebSocketRequest
}

export interface WebTransportCreatedEvent {
  transportId: RequestId
  url: string
  timestamp: MonotonicTime
  initiator?: Initiator
}

export interface WebTransportConnectionEstablishedEvent {
  transportId: RequestId
  timestamp: MonotonicTime
}

export interface WebTransportClosedEvent {
  transportId: RequestId
  timestamp: MonotonicTime
}

export interface DirectTCPSocketCreatedEvent {
  identifier: RequestId
  remoteAddr: string
  remotePort: number
  options: DirectTCPSocketOptions
  timestamp: MonotonicTime
  initiator?: Initiator
}

export interface DirectTCPSocketOpenedEvent {
  identifier: RequestId
  remoteAddr: string
  remotePort: number
  timestamp: MonotonicTime
  localAddr?: string
  localPort?: number
}

export interface DirectTCPSocketAbortedEvent {
  identifier: RequestId
  errorMessage: string
  timestamp: MonotonicTime
}

export interface DirectTCPSocketClosedEvent {
  identifier: RequestId
  timestamp: MonotonicTime
}

export interface DirectTCPSocketChunkSentEvent {
  identifier: RequestId
  data: string
  timestamp: MonotonicTime
}

export interface DirectTCPSocketChunkReceivedEvent {
  identifier: RequestId
  data: string
  timestamp: MonotonicTime
}

export interface DirectUDPSocketJoinedMulticastGroupEvent {
  identifier: RequestId
  IPAddress: string
}

export interface DirectUDPSocketLeftMulticastGroupEvent {
  identifier: RequestId
  IPAddress: string
}

export interface DirectUDPSocketCreatedEvent {
  identifier: RequestId
  options: DirectUDPSocketOptions
  timestamp: MonotonicTime
  initiator?: Initiator
}

export interface DirectUDPSocketOpenedEvent {
  identifier: RequestId
  localAddr: string
  localPort: number
  timestamp: MonotonicTime
  remoteAddr?: string
  remotePort?: number
}

export interface DirectUDPSocketAbortedEvent {
  identifier: RequestId
  errorMessage: string
  timestamp: MonotonicTime
}

export interface DirectUDPSocketClosedEvent {
  identifier: RequestId
  timestamp: MonotonicTime
}

export interface DirectUDPSocketChunkSentEvent {
  identifier: RequestId
  message: DirectUDPMessage
  timestamp: MonotonicTime
}

export interface DirectUDPSocketChunkReceivedEvent {
  identifier: RequestId
  message: DirectUDPMessage
  timestamp: MonotonicTime
}

export interface RequestWillBeSentExtraInfoEvent {
  requestId: RequestId
  associatedCookies: AssociatedCookie[]
  headers: Headers
  connectTiming: ConnectTiming
  deviceBoundSessionUsages?: DeviceBoundSessionWithUsage[]
  clientSecurityState?: ClientSecurityState
  siteHasCookieInOtherPartition?: boolean
  appliedNetworkConditionsId?: string
}

export interface ResponseReceivedExtraInfoEvent {
  requestId: RequestId
  blockedCookies: BlockedSetCookieWithReason[]
  headers: Headers
  resourceIPAddressSpace: IPAddressSpace
  statusCode: number
  headersText?: string
  cookiePartitionKey?: CookiePartitionKey
  cookiePartitionKeyOpaque?: boolean
  exemptedCookies?: ExemptedSetCookieWithReason[]
}

export interface ResponseReceivedEarlyHintsEvent {
  requestId: RequestId
  headers: Headers
}

export interface TrustTokenOperationDoneEvent {
  status:
    | 'Ok'
    | 'InvalidArgument'
    | 'MissingIssuerKeys'
    | 'FailedPrecondition'
    | 'ResourceExhausted'
    | 'AlreadyExists'
    | 'ResourceLimited'
    | 'Unauthorized'
    | 'BadResponse'
    | 'InternalError'
    | 'UnknownError'
    | 'FulfilledLocally'
    | 'SiteIssuerLimit'
  type: TrustTokenOperationType
  requestId: RequestId
  topLevelOrigin?: string
  issuerOrigin?: string
  issuedTokenCount?: number
}

export interface ReportingApiReportAddedEvent {
  report: ReportingApiReport
}

export interface ReportingApiReportUpdatedEvent {
  report: ReportingApiReport
}

export interface ReportingApiEndpointsChangedForOriginEvent {
  origin: string
  endpoints: ReportingApiEndpoint[]
}

export interface DeviceBoundSessionsAddedEvent {
  sessions: DeviceBoundSession[]
}

export interface DeviceBoundSessionEventOccurredEvent {
  eventId: DeviceBoundSessionEventId
  site: string
  succeeded: boolean
  sessionId?: string
  creationEventDetails?: CreationEventDetails
  refreshEventDetails?: RefreshEventDetails
  terminationEventDetails?: TerminationEventDetails
  challengeEventDetails?: ChallengeEventDetails
}
