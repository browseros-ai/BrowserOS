// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { BackendNodeId } from './dom'
import type {
  AdAncestry,
  ClientSecurityState,
  CorsErrorStatus,
  IPAddressSpace,
  LoaderId,
  RequestId,
} from './network'
import type { FrameId } from './page'
import type { ScriptId, StackTrace } from './runtime'

// ══ Types ══

export interface AffectedCookie {
  name: string
  path: string
  domain: string
}

export interface AffectedRequest {
  requestId?: RequestId
  url: string
}

export interface AffectedFrame {
  frameId: FrameId
}

export type CookieExclusionReason =
  | 'ExcludeSameSiteUnspecifiedTreatedAsLax'
  | 'ExcludeSameSiteNoneInsecure'
  | 'ExcludeSameSiteLax'
  | 'ExcludeSameSiteStrict'
  | 'ExcludeDomainNonASCII'
  | 'ExcludeThirdPartyCookieBlockedInFirstPartySet'
  | 'ExcludeThirdPartyPhaseout'
  | 'ExcludePortMismatch'
  | 'ExcludeSchemeMismatch'

export type CookieWarningReason =
  | 'WarnSameSiteUnspecifiedCrossSiteContext'
  | 'WarnSameSiteNoneInsecure'
  | 'WarnSameSiteUnspecifiedLaxAllowUnsafe'
  | 'WarnSameSiteStrictLaxDowngradeStrict'
  | 'WarnSameSiteStrictCrossDowngradeStrict'
  | 'WarnSameSiteStrictCrossDowngradeLax'
  | 'WarnSameSiteLaxCrossDowngradeStrict'
  | 'WarnSameSiteLaxCrossDowngradeLax'
  | 'WarnAttributeValueExceedsMaxSize'
  | 'WarnDomainNonASCII'
  | 'WarnThirdPartyPhaseout'
  | 'WarnCrossSiteRedirectDowngradeChangesInclusion'
  | 'WarnDeprecationTrialMetadata'
  | 'WarnThirdPartyCookieHeuristic'

export type CookieOperation = 'SetCookie' | 'ReadCookie'

export type InsightType = 'GitHubResource' | 'GracePeriod' | 'Heuristics'

export interface CookieIssueInsight {
  type: InsightType
  tableEntryUrl?: string
}

export interface CookieIssueDetails {
  cookie?: AffectedCookie
  rawCookieLine?: string
  cookieWarningReasons: CookieWarningReason[]
  cookieExclusionReasons: CookieExclusionReason[]
  operation: CookieOperation
  siteForCookies?: string
  cookieUrl?: string
  request?: AffectedRequest
  insight?: CookieIssueInsight
}

export type PerformanceIssueType = 'DocumentCookie'

export interface PerformanceIssueDetails {
  performanceIssueType: PerformanceIssueType
  sourceCodeLocation?: SourceCodeLocation
}

export type MixedContentResolutionStatus =
  | 'MixedContentBlocked'
  | 'MixedContentAutomaticallyUpgraded'
  | 'MixedContentWarning'

export type MixedContentResourceType =
  | 'Audio'
  | 'Beacon'
  | 'CSPReport'
  | 'Download'
  | 'EventSource'
  | 'Favicon'
  | 'Font'
  | 'Form'
  | 'Frame'
  | 'Image'
  | 'Import'
  | 'JSON'
  | 'Manifest'
  | 'Ping'
  | 'PluginData'
  | 'PluginResource'
  | 'Prefetch'
  | 'Resource'
  | 'Script'
  | 'ServiceWorker'
  | 'SharedWorker'
  | 'SpeculationRules'
  | 'Stylesheet'
  | 'Track'
  | 'Video'
  | 'Worker'
  | 'XMLHttpRequest'
  | 'XSLT'

export interface MixedContentIssueDetails {
  resourceType?: MixedContentResourceType
  resolutionStatus: MixedContentResolutionStatus
  insecureURL: string
  mainResourceURL: string
  request?: AffectedRequest
  frame?: AffectedFrame
}

export type BlockedByResponseReason =
  | 'CoepFrameResourceNeedsCoepHeader'
  | 'CoopSandboxedIFrameCannotNavigateToCoopPage'
  | 'CorpNotSameOrigin'
  | 'CorpNotSameOriginAfterDefaultedToSameOriginByCoep'
  | 'CorpNotSameOriginAfterDefaultedToSameOriginByDip'
  | 'CorpNotSameOriginAfterDefaultedToSameOriginByCoepAndDip'
  | 'CorpNotSameSite'
  | 'SRIMessageSignatureMismatch'

export interface BlockedByResponseIssueDetails {
  request: AffectedRequest
  parentFrame?: AffectedFrame
  blockedFrame?: AffectedFrame
  reason: BlockedByResponseReason
}

export type HeavyAdResolutionStatus = 'HeavyAdBlocked' | 'HeavyAdWarning'

export type HeavyAdReason =
  | 'NetworkTotalLimit'
  | 'CpuTotalLimit'
  | 'CpuPeakLimit'

export interface HeavyAdIssueDetails {
  resolution: HeavyAdResolutionStatus
  reason: HeavyAdReason
  frame: AffectedFrame
}

export type ContentSecurityPolicyViolationType =
  | 'kInlineViolation'
  | 'kEvalViolation'
  | 'kURLViolation'
  | 'kSRIViolation'
  | 'kTrustedTypesSinkViolation'
  | 'kTrustedTypesPolicyViolation'
  | 'kWasmEvalViolation'

export interface SourceCodeLocation {
  scriptId?: ScriptId
  url: string
  lineNumber: number
  columnNumber: number
}

export interface ContentSecurityPolicyIssueDetails {
  blockedURL?: string
  violatedDirective: string
  isReportOnly: boolean
  contentSecurityPolicyViolationType: ContentSecurityPolicyViolationType
  frameAncestor?: AffectedFrame
  sourceCodeLocation?: SourceCodeLocation
  violatingNodeId?: BackendNodeId
}

export type SharedArrayBufferIssueType = 'TransferIssue' | 'CreationIssue'

export interface SharedArrayBufferIssueDetails {
  sourceCodeLocation: SourceCodeLocation
  isWarning: boolean
  type: SharedArrayBufferIssueType
}

export interface CorsIssueDetails {
  corsErrorStatus: CorsErrorStatus
  isWarning: boolean
  request: AffectedRequest
  location?: SourceCodeLocation
  initiatorOrigin?: string
  resourceIPAddressSpace?: IPAddressSpace
  clientSecurityState?: ClientSecurityState
}

export type SharedDictionaryError =
  | 'UseErrorCrossOriginNoCorsRequest'
  | 'UseErrorDictionaryLoadFailure'
  | 'UseErrorMatchingDictionaryNotUsed'
  | 'UseErrorUnexpectedContentDictionaryHeader'
  | 'WriteErrorCossOriginNoCorsRequest'
  | 'WriteErrorDisallowedBySettings'
  | 'WriteErrorExpiredResponse'
  | 'WriteErrorFeatureDisabled'
  | 'WriteErrorInsufficientResources'
  | 'WriteErrorInvalidMatchField'
  | 'WriteErrorInvalidStructuredHeader'
  | 'WriteErrorInvalidTTLField'
  | 'WriteErrorNavigationRequest'
  | 'WriteErrorNoMatchField'
  | 'WriteErrorNonIntegerTTLField'
  | 'WriteErrorNonListMatchDestField'
  | 'WriteErrorNonSecureContext'
  | 'WriteErrorNonStringIdField'
  | 'WriteErrorNonStringInMatchDestList'
  | 'WriteErrorInvalidMatchDestList'
  | 'WriteErrorNonStringMatchField'
  | 'WriteErrorNonTokenTypeField'
  | 'WriteErrorRequestAborted'
  | 'WriteErrorShuttingDown'
  | 'WriteErrorTooLongIdField'
  | 'WriteErrorUnsupportedType'

export type SRIMessageSignatureError =
  | 'MissingSignatureHeader'
  | 'MissingSignatureInputHeader'
  | 'InvalidSignatureHeader'
  | 'InvalidSignatureInputHeader'
  | 'SignatureHeaderValueIsNotByteSequence'
  | 'SignatureHeaderValueIsParameterized'
  | 'SignatureHeaderValueIsIncorrectLength'
  | 'SignatureInputHeaderMissingLabel'
  | 'SignatureInputHeaderValueNotInnerList'
  | 'SignatureInputHeaderValueMissingComponents'
  | 'SignatureInputHeaderInvalidComponentType'
  | 'SignatureInputHeaderInvalidComponentName'
  | 'SignatureInputHeaderInvalidHeaderComponentParameter'
  | 'SignatureInputHeaderInvalidDerivedComponentParameter'
  | 'SignatureInputHeaderKeyIdLength'
  | 'SignatureInputHeaderInvalidParameter'
  | 'SignatureInputHeaderMissingRequiredParameters'
  | 'ValidationFailedSignatureExpired'
  | 'ValidationFailedInvalidLength'
  | 'ValidationFailedSignatureMismatch'
  | 'ValidationFailedIntegrityMismatch'
  | 'SignatureBaseUnknownDerivedComponent'
  | 'SignatureBaseMissingHeader'
  | 'SignatureBaseInvalidUnencodedDigest'
  | 'SignatureBaseUnsupportedComponent'

export type UnencodedDigestError =
  | 'MalformedDictionary'
  | 'UnknownAlgorithm'
  | 'IncorrectDigestType'
  | 'IncorrectDigestLength'

export type ConnectionAllowlistError =
  | 'InvalidHeader'
  | 'MoreThanOneList'
  | 'ItemNotInnerList'
  | 'InvalidAllowlistItemType'
  | 'ReportingEndpointNotToken'
  | 'InvalidUrlPattern'
  | 'IFrameAttributeLoosensEmbeddingRequirement'
  | 'InvalidAllowConnectionAllowlistFrom'
  | 'EmbeddingRequirementNotSatisfied'

export interface QuirksModeIssueDetails {
  isLimitedQuirksMode: boolean
  documentNodeId: BackendNodeId
  url: string
  frameId: FrameId
  loaderId: LoaderId
}

export interface NavigatorUserAgentIssueDetails {
  url: string
  location?: SourceCodeLocation
}

export interface SharedDictionaryIssueDetails {
  sharedDictionaryError: SharedDictionaryError
  request: AffectedRequest
}

export interface SRIMessageSignatureIssueDetails {
  error: SRIMessageSignatureError
  signatureBase: string
  integrityAssertions: string[]
  request: AffectedRequest
}

export interface UnencodedDigestIssueDetails {
  error: UnencodedDigestError
  request: AffectedRequest
}

export interface ConnectionAllowlistIssueDetails {
  error: ConnectionAllowlistError
  request: AffectedRequest
}

export type GenericIssueErrorType =
  | 'FormLabelForNameError'
  | 'FormDuplicateIdForInputError'
  | 'FormInputWithNoLabelError'
  | 'FormAutocompleteAttributeEmptyError'
  | 'FormEmptyIdAndNameAttributesForInputError'
  | 'FormAriaLabelledByToNonExistingIdError'
  | 'FormInputAssignedAutocompleteValueToIdOrNameAttributeError'
  | 'FormLabelHasNeitherForNorNestedInputError'
  | 'FormLabelForMatchesNonExistingIdError'
  | 'FormInputHasWrongButWellIntendedAutocompleteValueError'
  | 'ResponseWasBlockedByORB'
  | 'NavigationEntryMarkedSkippable'
  | 'BackUINavigationWouldSkipAd'
  | 'AutofillAndManualTextPolicyControlledFeaturesInfo'
  | 'AutofillPolicyControlledFeatureInfo'
  | 'ManualTextPolicyControlledFeatureInfo'
  | 'FormModelContextParameterMissingTitleAndDescription'
  | 'FormModelContextMissingToolName'
  | 'FormModelContextMissingToolDescription'
  | 'FormModelContextRequiredParameterMissingName'
  | 'FormModelContextParameterMissingName'

export interface GenericIssueDetails {
  errorType: GenericIssueErrorType
  frameId?: FrameId
  violatingNodeId?: BackendNodeId
  violatingNodeAttribute?: string
  request?: AffectedRequest
}

export interface DeprecationIssueDetails {
  affectedFrame?: AffectedFrame
  sourceCodeLocation: SourceCodeLocation
  type: string
}

export interface BounceTrackingIssueDetails {
  trackingSites: string[]
}

export interface CookieDeprecationMetadataIssueDetails {
  allowedSites: string[]
  optOutPercentage: number
  isOptOutTopLevel: boolean
  operation: CookieOperation
}

export type ClientHintIssueReason =
  | 'MetaTagAllowListInvalidOrigin'
  | 'MetaTagModifiedHTML'

export interface FederatedAuthRequestIssueDetails {
  federatedAuthRequestIssueReason: FederatedAuthRequestIssueReason
}

export type FederatedAuthRequestIssueReason =
  | 'ShouldEmbargo'
  | 'TooManyRequests'
  | 'WellKnownHttpNotFound'
  | 'WellKnownNoResponse'
  | 'WellKnownBlockedByConnectionAllowlist'
  | 'WellKnownInvalidResponse'
  | 'WellKnownListEmpty'
  | 'WellKnownInvalidContentType'
  | 'ConfigNotInWellKnown'
  | 'WellKnownTooBig'
  | 'ConfigHttpNotFound'
  | 'ConfigNoResponse'
  | 'ConfigBlockedByConnectionAllowlist'
  | 'ConfigInvalidResponse'
  | 'ConfigInvalidContentType'
  | 'IdpNotPotentiallyTrustworthy'
  | 'DisabledInSettings'
  | 'DisabledInFlags'
  | 'ErrorFetchingSignin'
  | 'InvalidSigninResponse'
  | 'AccountsHttpNotFound'
  | 'AccountsNoResponse'
  | 'AccountsBlockedByConnectionAllowlist'
  | 'AccountsInvalidResponse'
  | 'AccountsListEmpty'
  | 'AccountsInvalidContentType'
  | 'IdTokenHttpNotFound'
  | 'IdTokenNoResponse'
  | 'IdTokenBlockedByConnectionAllowlist'
  | 'IdTokenInvalidResponse'
  | 'IdTokenIdpErrorResponse'
  | 'IdTokenCrossSiteIdpErrorResponse'
  | 'IdTokenInvalidRequest'
  | 'IdTokenInvalidContentType'
  | 'ErrorIdToken'
  | 'Canceled'
  | 'RpPageNotVisible'
  | 'SilentMediationFailure'
  | 'NotSignedInWithIdp'
  | 'MissingTransientUserActivation'
  | 'ReplacedByActiveMode'
  | 'RelyingPartyOriginIsOpaque'
  | 'TypeNotMatching'
  | 'UiDismissedNoEmbargo'
  | 'CorsError'
  | 'SuppressedBySegmentationPlatform'
  | 'PopupBlockedByConnectionAllowlist'

export interface FederatedAuthUserInfoRequestIssueDetails {
  federatedAuthUserInfoRequestIssueReason: FederatedAuthUserInfoRequestIssueReason
}

export type FederatedAuthUserInfoRequestIssueReason =
  | 'NotSameOrigin'
  | 'NotIframe'
  | 'NotPotentiallyTrustworthy'
  | 'NoApiPermission'
  | 'NotSignedInWithIdp'
  | 'NoAccountSharingPermission'
  | 'InvalidConfigOrWellKnown'
  | 'InvalidAccountsResponse'
  | 'NoReturningUserFromFetchedAccounts'

export interface EmailVerificationRequestIssueDetails {
  emailVerificationRequestIssueReason: EmailVerificationRequestIssueReason
}

export type EmailVerificationRequestIssueReason =
  | 'InvalidEmail'
  | 'DnsFetchFailed'
  | 'DnsInvalidRecord'
  | 'WellKnownHttpNotFound'
  | 'WellKnownNoResponse'
  | 'WellKnownInvalidResponse'
  | 'WellKnownListEmpty'
  | 'WellKnownInvalidContentType'
  | 'WellKnownMissingIssuanceEndpoint'
  | 'WellKnownIssuanceEndpointCrossOrigin'
  | 'WellKnownUnsupportedSigningAlgorithm'
  | 'TokenHttpNotFound'
  | 'TokenNoResponse'
  | 'TokenInvalidResponse'
  | 'TokenInvalidContentType'
  | 'TokenMalformedSdJwt'
  | 'TokenInvalidSdJwt'
  | 'KeyBindingSigningFailed'
  | 'RpOriginIsOpaque'
  | 'WellKnownMissingAccountsEndpoint'
  | 'UserLoggedOut'
  | 'WellKnownAccountsEndpointCrossOrigin'
  | 'AccountsHttpNotFound'
  | 'AccountsNoResponse'
  | 'AccountsInvalidResponse'
  | 'AccountsInvalidContentType'
  | 'AccountsEmptyList'
  | 'EmailVerificationWellKnownHttpNotFound'
  | 'EmailVerificationWellKnownNoResponse'
  | 'EmailVerificationWellKnownInvalidResponse'
  | 'EmailVerificationWellKnownInvalidContentType'
  | 'JwksHttpNotFound'
  | 'JwksInvalidResponse'
  | 'TokenVerificationSdJwtUnsupportedHeaderAlg'
  | 'TokenVerificationSdJwtInvalidTyp'
  | 'TokenVerificationSdJwtMissingIss'
  | 'TokenVerificationSdJwtMissingIat'
  | 'TokenVerificationSdJwtMissingCnf'
  | 'TokenVerificationSdJwtMissingEmail'
  | 'TokenVerificationSdJwtInvalidIssuedAt'
  | 'TokenVerificationSdJwtInvalidIssuer'
  | 'TokenVerificationSdJwtJwksMissingKeys'
  | 'TokenVerificationSdJwtSignatureFailed'
  | 'TokenVerificationSdJwtInvalidEmailVerified'
  | 'TokenVerificationSdJwtInvalidEmail'
  | 'TokenVerificationSdJwtInvalidHolderKey'
  | 'TokenVerificationKbInvalidTyp'
  | 'TokenVerificationKbMissingAud'
  | 'TokenVerificationKbMissingNonce'
  | 'TokenVerificationKbMissingIat'
  | 'TokenVerificationKbMissingSdHash'
  | 'TokenVerificationKbInvalidIssuedAt'
  | 'TokenVerificationKbInvalidAudience'
  | 'TokenVerificationKbInvalidNonce'
  | 'TokenVerificationKbInvalidSdHash'
  | 'TokenVerificationKbMissingCnf'
  | 'TokenVerificationKbSignatureFailed'
  | 'CrossOriginIframeNotSupported'

export interface ClientHintIssueDetails {
  sourceCodeLocation: SourceCodeLocation
  clientHintIssueReason: ClientHintIssueReason
}

export interface FailedRequestInfo {
  url: string
  failureMessage: string
  requestId?: RequestId
}

export type PartitioningBlobURLInfo =
  | 'BlockedCrossPartitionFetching'
  | 'EnforceNoopenerForNavigation'

export interface PartitioningBlobURLIssueDetails {
  url: string
  partitioningBlobURLInfo: PartitioningBlobURLInfo
}

export type ElementAccessibilityIssueReason =
  | 'DisallowedSelectChild'
  | 'DisallowedOptGroupChild'
  | 'NonPhrasingContentOptionChild'
  | 'InteractiveContentOptionChild'
  | 'InteractiveContentLegendChild'
  | 'InteractiveContentSummaryDescendant'

export interface ElementAccessibilityIssueDetails {
  nodeId: BackendNodeId
  elementAccessibilityIssueReason: ElementAccessibilityIssueReason
  hasDisallowedAttributes: boolean
}

export type StyleSheetLoadingIssueReason = 'LateImportRule' | 'RequestFailed'

export interface StylesheetLoadingIssueDetails {
  sourceCodeLocation: SourceCodeLocation
  styleSheetLoadingIssueReason: StyleSheetLoadingIssueReason
  failedRequestInfo?: FailedRequestInfo
}

export type PropertyRuleIssueReason =
  | 'InvalidSyntax'
  | 'InvalidInitialValue'
  | 'InvalidInherits'
  | 'InvalidName'

export interface PropertyRuleIssueDetails {
  sourceCodeLocation: SourceCodeLocation
  propertyRuleIssueReason: PropertyRuleIssueReason
  propertyValue?: string
}

export type UserReidentificationIssueType =
  | 'BlockedFrameNavigation'
  | 'BlockedSubresource'
  | 'NoisedCanvasReadback'

export interface UserReidentificationIssueDetails {
  type: UserReidentificationIssueType
  request?: AffectedRequest
  sourceCodeLocation?: SourceCodeLocation
}

export type PermissionElementIssueType =
  | 'InvalidType'
  | 'FencedFrameDisallowed'
  | 'CspFrameAncestorsMissing'
  | 'PermissionsPolicyBlocked'
  | 'PaddingRightUnsupported'
  | 'PaddingBottomUnsupported'
  | 'InsetBoxShadowUnsupported'
  | 'RequestInProgress'
  | 'UntrustedEvent'
  | 'RegistrationFailed'
  | 'TypeNotSupported'
  | 'InvalidTypeActivation'
  | 'SecurityChecksFailed'
  | 'ActivationDisabled'
  | 'GeolocationDeprecated'
  | 'InvalidDisplayStyle'
  | 'NonOpaqueColor'
  | 'LowContrast'
  | 'FontSizeTooSmall'
  | 'FontSizeTooLarge'
  | 'InvalidSizeValue'
  | 'NonSecureContext'
  | 'MissingTransientUserActivation'

export interface PermissionElementIssueDetails {
  issueType: PermissionElementIssueType
  type?: string
  nodeId?: BackendNodeId
  isWarning?: boolean
  permissionName?: string
  occluderNodeInfo?: string
  occluderParentNodeInfo?: string
  disableReason?: string
}

export type WebInstallIssueReason =
  | 'ManifestParsingOrNetworkError'
  | 'StartUrlInvalid'
  | 'ManifestMissingNameOrShortName'
  | 'ManifestMissingId'
  | 'NoManifest'

export interface WebInstallIssueDetails {
  manifestUrl?: string
  reason: WebInstallIssueReason
}

export interface SelectivePermissionsInterventionIssueDetails {
  apiName: string
  adAncestry: AdAncestry
  stackTrace?: StackTrace
}

export interface LazyLoadImageIssueDetails {
  nodeId: BackendNodeId
  url: string
  frameId: FrameId
}

export type InspectorIssueCode =
  | 'CookieIssue'
  | 'MixedContentIssue'
  | 'BlockedByResponseIssue'
  | 'HeavyAdIssue'
  | 'ContentSecurityPolicyIssue'
  | 'SharedArrayBufferIssue'
  | 'CorsIssue'
  | 'QuirksModeIssue'
  | 'PartitioningBlobURLIssue'
  | 'NavigatorUserAgentIssue'
  | 'GenericIssue'
  | 'DeprecationIssue'
  | 'ClientHintIssue'
  | 'FederatedAuthRequestIssue'
  | 'BounceTrackingIssue'
  | 'CookieDeprecationMetadataIssue'
  | 'StylesheetLoadingIssue'
  | 'FederatedAuthUserInfoRequestIssue'
  | 'PropertyRuleIssue'
  | 'SharedDictionaryIssue'
  | 'ElementAccessibilityIssue'
  | 'SRIMessageSignatureIssue'
  | 'UnencodedDigestIssue'
  | 'ConnectionAllowlistIssue'
  | 'UserReidentificationIssue'
  | 'PermissionElementIssue'
  | 'PerformanceIssue'
  | 'SelectivePermissionsInterventionIssue'
  | 'EmailVerificationRequestIssue'
  | 'LazyLoadImageIssue'
  | 'WebInstallIssue'

export interface InspectorIssueDetails {
  cookieIssueDetails?: CookieIssueDetails
  mixedContentIssueDetails?: MixedContentIssueDetails
  blockedByResponseIssueDetails?: BlockedByResponseIssueDetails
  heavyAdIssueDetails?: HeavyAdIssueDetails
  contentSecurityPolicyIssueDetails?: ContentSecurityPolicyIssueDetails
  sharedArrayBufferIssueDetails?: SharedArrayBufferIssueDetails
  corsIssueDetails?: CorsIssueDetails
  quirksModeIssueDetails?: QuirksModeIssueDetails
  partitioningBlobURLIssueDetails?: PartitioningBlobURLIssueDetails
  navigatorUserAgentIssueDetails?: NavigatorUserAgentIssueDetails
  genericIssueDetails?: GenericIssueDetails
  deprecationIssueDetails?: DeprecationIssueDetails
  clientHintIssueDetails?: ClientHintIssueDetails
  federatedAuthRequestIssueDetails?: FederatedAuthRequestIssueDetails
  bounceTrackingIssueDetails?: BounceTrackingIssueDetails
  cookieDeprecationMetadataIssueDetails?: CookieDeprecationMetadataIssueDetails
  stylesheetLoadingIssueDetails?: StylesheetLoadingIssueDetails
  propertyRuleIssueDetails?: PropertyRuleIssueDetails
  federatedAuthUserInfoRequestIssueDetails?: FederatedAuthUserInfoRequestIssueDetails
  sharedDictionaryIssueDetails?: SharedDictionaryIssueDetails
  elementAccessibilityIssueDetails?: ElementAccessibilityIssueDetails
  sriMessageSignatureIssueDetails?: SRIMessageSignatureIssueDetails
  unencodedDigestIssueDetails?: UnencodedDigestIssueDetails
  connectionAllowlistIssueDetails?: ConnectionAllowlistIssueDetails
  userReidentificationIssueDetails?: UserReidentificationIssueDetails
  permissionElementIssueDetails?: PermissionElementIssueDetails
  performanceIssueDetails?: PerformanceIssueDetails
  selectivePermissionsInterventionIssueDetails?: SelectivePermissionsInterventionIssueDetails
  emailVerificationRequestIssueDetails?: EmailVerificationRequestIssueDetails
  lazyLoadImageIssueDetails?: LazyLoadImageIssueDetails
  webInstallIssueDetails?: WebInstallIssueDetails
}

export type IssueId = string

export interface InspectorIssue {
  code: InspectorIssueCode
  details: InspectorIssueDetails
  issueId?: IssueId
}

// ══ Commands ══

export interface GetEncodedResponseParams {
  requestId: RequestId
  encoding: 'webp' | 'jpeg' | 'png'
  quality?: number
  sizeOnly?: boolean
}

export interface GetEncodedResponseResult {
  body?: string
  originalSize: number
  encodedSize: number
}

export interface CheckFormsIssuesResult {
  formIssues: GenericIssueDetails[]
}

// ══ Events ══

export interface IssueAddedEvent {
  issue: InspectorIssue
}
