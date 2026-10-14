// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  CanClearBrowserCacheResult,
  CanClearBrowserCookiesResult,
  CanEmulateNetworkConditionsResult,
  ConfigureDurableMessagesParams,
  DataReceivedEvent,
  DeleteCookiesParams,
  DeleteDeviceBoundSessionParams,
  DeviceBoundSessionEventOccurredEvent,
  DeviceBoundSessionsAddedEvent,
  DirectTCPSocketAbortedEvent,
  DirectTCPSocketChunkReceivedEvent,
  DirectTCPSocketChunkSentEvent,
  DirectTCPSocketClosedEvent,
  DirectTCPSocketCreatedEvent,
  DirectTCPSocketOpenedEvent,
  DirectUDPSocketAbortedEvent,
  DirectUDPSocketChunkReceivedEvent,
  DirectUDPSocketChunkSentEvent,
  DirectUDPSocketClosedEvent,
  DirectUDPSocketCreatedEvent,
  DirectUDPSocketJoinedMulticastGroupEvent,
  DirectUDPSocketLeftMulticastGroupEvent,
  DirectUDPSocketOpenedEvent,
  EmulateNetworkConditionsByRuleParams,
  EmulateNetworkConditionsByRuleResult,
  EmulateNetworkConditionsParams,
  EnableDeviceBoundSessionsParams,
  EnableParams,
  EnableReportingApiParams,
  EventSourceMessageReceivedEvent,
  FetchSchemefulSiteParams,
  FetchSchemefulSiteResult,
  GetAllCookiesResult,
  GetCertificateParams,
  GetCertificateResult,
  GetCookiesParams,
  GetCookiesResult,
  GetRequestPostDataParams,
  GetRequestPostDataResult,
  GetResponseBodyParams,
  GetResponseBodyResult,
  GetSecurityIsolationStatusParams,
  GetSecurityIsolationStatusResult,
  LoadingFailedEvent,
  LoadingFinishedEvent,
  LoadNetworkResourceParams,
  LoadNetworkResourceResult,
  OverrideNetworkStateParams,
  ReplayXHRParams,
  ReportingApiEndpointsChangedForOriginEvent,
  ReportingApiReportAddedEvent,
  ReportingApiReportUpdatedEvent,
  RequestServedFromCacheEvent,
  RequestWillBeSentEvent,
  RequestWillBeSentExtraInfoEvent,
  ResourceChangedPriorityEvent,
  ResponseReceivedEarlyHintsEvent,
  ResponseReceivedEvent,
  ResponseReceivedExtraInfoEvent,
  SearchInResponseBodyParams,
  SearchInResponseBodyResult,
  SetAttachDebugStackParams,
  SetBlockedURLsParams,
  SetBypassServiceWorkerParams,
  SetCacheDisabledParams,
  SetCookieControlsParams,
  SetCookieParams,
  SetCookieResult,
  SetCookiesParams,
  SetExtraHTTPHeadersParams,
  SetUserAgentOverrideParams,
  SignedExchangeReceivedEvent,
  StreamResourceContentParams,
  StreamResourceContentResult,
  TrustTokenOperationDoneEvent,
  WebSocketClosedEvent,
  WebSocketCreatedEvent,
  WebSocketFrameErrorEvent,
  WebSocketFrameReceivedEvent,
  WebSocketFrameSentEvent,
  WebSocketHandshakeResponseReceivedEvent,
  WebSocketWillSendHandshakeRequestEvent,
  WebTransportClosedEvent,
  WebTransportConnectionEstablishedEvent,
  WebTransportCreatedEvent,
} from '../domains/network'

export interface NetworkApi {
  // ── Commands ──

  canClearBrowserCache(): Promise<CanClearBrowserCacheResult>
  canClearBrowserCookies(): Promise<CanClearBrowserCookiesResult>
  canEmulateNetworkConditions(): Promise<CanEmulateNetworkConditionsResult>
  clearBrowserCache(): Promise<void>
  clearBrowserCookies(): Promise<void>
  deleteCookies(params: DeleteCookiesParams): Promise<void>
  disable(): Promise<void>
  emulateNetworkConditions(
    params: EmulateNetworkConditionsParams,
  ): Promise<void>
  emulateNetworkConditionsByRule(
    params: EmulateNetworkConditionsByRuleParams,
  ): Promise<EmulateNetworkConditionsByRuleResult>
  overrideNetworkState(params: OverrideNetworkStateParams): Promise<void>
  enable(params?: EnableParams): Promise<void>
  configureDurableMessages(
    params?: ConfigureDurableMessagesParams,
  ): Promise<void>
  getAllCookies(): Promise<GetAllCookiesResult>
  getCertificate(params: GetCertificateParams): Promise<GetCertificateResult>
  getCookies(params?: GetCookiesParams): Promise<GetCookiesResult>
  getResponseBody(params: GetResponseBodyParams): Promise<GetResponseBodyResult>
  getRequestPostData(
    params: GetRequestPostDataParams,
  ): Promise<GetRequestPostDataResult>
  replayXHR(params: ReplayXHRParams): Promise<void>
  searchInResponseBody(
    params: SearchInResponseBodyParams,
  ): Promise<SearchInResponseBodyResult>
  setBlockedURLs(params?: SetBlockedURLsParams): Promise<void>
  setBypassServiceWorker(params: SetBypassServiceWorkerParams): Promise<void>
  setCacheDisabled(params: SetCacheDisabledParams): Promise<void>
  setCookie(params: SetCookieParams): Promise<SetCookieResult>
  setCookies(params: SetCookiesParams): Promise<void>
  setExtraHTTPHeaders(params: SetExtraHTTPHeadersParams): Promise<void>
  setAttachDebugStack(params: SetAttachDebugStackParams): Promise<void>
  setUserAgentOverride(params: SetUserAgentOverrideParams): Promise<void>
  streamResourceContent(
    params: StreamResourceContentParams,
  ): Promise<StreamResourceContentResult>
  getSecurityIsolationStatus(
    params?: GetSecurityIsolationStatusParams,
  ): Promise<GetSecurityIsolationStatusResult>
  enableReportingApi(params: EnableReportingApiParams): Promise<void>
  enableDeviceBoundSessions(
    params: EnableDeviceBoundSessionsParams,
  ): Promise<void>
  deleteDeviceBoundSession(
    params: DeleteDeviceBoundSessionParams,
  ): Promise<void>
  fetchSchemefulSite(
    params: FetchSchemefulSiteParams,
  ): Promise<FetchSchemefulSiteResult>
  loadNetworkResource(
    params: LoadNetworkResourceParams,
  ): Promise<LoadNetworkResourceResult>
  setCookieControls(params: SetCookieControlsParams): Promise<void>

  // ── Events ──

  on(
    event: 'dataReceived',
    handler: (params: DataReceivedEvent) => void,
  ): () => void
  on(
    event: 'eventSourceMessageReceived',
    handler: (params: EventSourceMessageReceivedEvent) => void,
  ): () => void
  on(
    event: 'loadingFailed',
    handler: (params: LoadingFailedEvent) => void,
  ): () => void
  on(
    event: 'loadingFinished',
    handler: (params: LoadingFinishedEvent) => void,
  ): () => void
  on(
    event: 'requestServedFromCache',
    handler: (params: RequestServedFromCacheEvent) => void,
  ): () => void
  on(
    event: 'requestWillBeSent',
    handler: (params: RequestWillBeSentEvent) => void,
  ): () => void
  on(
    event: 'resourceChangedPriority',
    handler: (params: ResourceChangedPriorityEvent) => void,
  ): () => void
  on(
    event: 'signedExchangeReceived',
    handler: (params: SignedExchangeReceivedEvent) => void,
  ): () => void
  on(
    event: 'responseReceived',
    handler: (params: ResponseReceivedEvent) => void,
  ): () => void
  on(
    event: 'webSocketClosed',
    handler: (params: WebSocketClosedEvent) => void,
  ): () => void
  on(
    event: 'webSocketCreated',
    handler: (params: WebSocketCreatedEvent) => void,
  ): () => void
  on(
    event: 'webSocketFrameError',
    handler: (params: WebSocketFrameErrorEvent) => void,
  ): () => void
  on(
    event: 'webSocketFrameReceived',
    handler: (params: WebSocketFrameReceivedEvent) => void,
  ): () => void
  on(
    event: 'webSocketFrameSent',
    handler: (params: WebSocketFrameSentEvent) => void,
  ): () => void
  on(
    event: 'webSocketHandshakeResponseReceived',
    handler: (params: WebSocketHandshakeResponseReceivedEvent) => void,
  ): () => void
  on(
    event: 'webSocketWillSendHandshakeRequest',
    handler: (params: WebSocketWillSendHandshakeRequestEvent) => void,
  ): () => void
  on(
    event: 'webTransportCreated',
    handler: (params: WebTransportCreatedEvent) => void,
  ): () => void
  on(
    event: 'webTransportConnectionEstablished',
    handler: (params: WebTransportConnectionEstablishedEvent) => void,
  ): () => void
  on(
    event: 'webTransportClosed',
    handler: (params: WebTransportClosedEvent) => void,
  ): () => void
  on(
    event: 'directTCPSocketCreated',
    handler: (params: DirectTCPSocketCreatedEvent) => void,
  ): () => void
  on(
    event: 'directTCPSocketOpened',
    handler: (params: DirectTCPSocketOpenedEvent) => void,
  ): () => void
  on(
    event: 'directTCPSocketAborted',
    handler: (params: DirectTCPSocketAbortedEvent) => void,
  ): () => void
  on(
    event: 'directTCPSocketClosed',
    handler: (params: DirectTCPSocketClosedEvent) => void,
  ): () => void
  on(
    event: 'directTCPSocketChunkSent',
    handler: (params: DirectTCPSocketChunkSentEvent) => void,
  ): () => void
  on(
    event: 'directTCPSocketChunkReceived',
    handler: (params: DirectTCPSocketChunkReceivedEvent) => void,
  ): () => void
  on(
    event: 'directUDPSocketJoinedMulticastGroup',
    handler: (params: DirectUDPSocketJoinedMulticastGroupEvent) => void,
  ): () => void
  on(
    event: 'directUDPSocketLeftMulticastGroup',
    handler: (params: DirectUDPSocketLeftMulticastGroupEvent) => void,
  ): () => void
  on(
    event: 'directUDPSocketCreated',
    handler: (params: DirectUDPSocketCreatedEvent) => void,
  ): () => void
  on(
    event: 'directUDPSocketOpened',
    handler: (params: DirectUDPSocketOpenedEvent) => void,
  ): () => void
  on(
    event: 'directUDPSocketAborted',
    handler: (params: DirectUDPSocketAbortedEvent) => void,
  ): () => void
  on(
    event: 'directUDPSocketClosed',
    handler: (params: DirectUDPSocketClosedEvent) => void,
  ): () => void
  on(
    event: 'directUDPSocketChunkSent',
    handler: (params: DirectUDPSocketChunkSentEvent) => void,
  ): () => void
  on(
    event: 'directUDPSocketChunkReceived',
    handler: (params: DirectUDPSocketChunkReceivedEvent) => void,
  ): () => void
  on(
    event: 'requestWillBeSentExtraInfo',
    handler: (params: RequestWillBeSentExtraInfoEvent) => void,
  ): () => void
  on(
    event: 'responseReceivedExtraInfo',
    handler: (params: ResponseReceivedExtraInfoEvent) => void,
  ): () => void
  on(
    event: 'responseReceivedEarlyHints',
    handler: (params: ResponseReceivedEarlyHintsEvent) => void,
  ): () => void
  on(
    event: 'trustTokenOperationDone',
    handler: (params: TrustTokenOperationDoneEvent) => void,
  ): () => void
  on(event: 'policyUpdated', handler: () => void): () => void
  on(
    event: 'reportingApiReportAdded',
    handler: (params: ReportingApiReportAddedEvent) => void,
  ): () => void
  on(
    event: 'reportingApiReportUpdated',
    handler: (params: ReportingApiReportUpdatedEvent) => void,
  ): () => void
  on(
    event: 'reportingApiEndpointsChangedForOrigin',
    handler: (params: ReportingApiEndpointsChangedForOriginEvent) => void,
  ): () => void
  on(
    event: 'deviceBoundSessionsAdded',
    handler: (params: DeviceBoundSessionsAddedEvent) => void,
  ): () => void
  on(
    event: 'deviceBoundSessionEventOccurred',
    handler: (params: DeviceBoundSessionEventOccurredEvent) => void,
  ): () => void
}
