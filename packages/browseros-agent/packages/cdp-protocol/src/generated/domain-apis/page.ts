// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  AddCompilationCacheParams,
  AddScriptToEvaluateOnLoadParams,
  AddScriptToEvaluateOnLoadResult,
  AddScriptToEvaluateOnNewDocumentParams,
  AddScriptToEvaluateOnNewDocumentResult,
  BackForwardCacheNotUsedEvent,
  CaptureScreenshotParams,
  CaptureScreenshotResult,
  CaptureSnapshotParams,
  CaptureSnapshotResult,
  CompilationCacheProducedEvent,
  CreateIsolatedWorldParams,
  CreateIsolatedWorldResult,
  DeleteCookieParams,
  DocumentOpenedEvent,
  DomContentEventFiredEvent,
  DownloadProgressEvent,
  DownloadWillBeginEvent,
  EnableParams,
  FileChooserOpenedEvent,
  FrameAttachedEvent,
  FrameClearedScheduledNavigationEvent,
  FrameDetachedEvent,
  FrameNavigatedEvent,
  FrameRequestedNavigationEvent,
  FrameScheduledNavigationEvent,
  FrameStartedLoadingEvent,
  FrameStartedNavigatingEvent,
  FrameStoppedLoadingEvent,
  FrameSubtreeWillBeDetachedEvent,
  GenerateTestReportParams,
  GetAdScriptAncestryParams,
  GetAdScriptAncestryResult,
  GetAnnotatedPageContentParams,
  GetAnnotatedPageContentResult,
  GetAppIdResult,
  GetAppManifestParams,
  GetAppManifestResult,
  GetFrameTreeResult,
  GetInstallabilityErrorsResult,
  GetLayoutMetricsResult,
  GetManifestIconsResult,
  GetNavigationHistoryResult,
  GetOriginTrialsParams,
  GetOriginTrialsResult,
  GetPermissionsPolicyStateParams,
  GetPermissionsPolicyStateResult,
  GetResourceContentParams,
  GetResourceContentResult,
  GetResourceTreeResult,
  HandleJavaScriptDialogParams,
  JavascriptDialogClosedEvent,
  JavascriptDialogOpeningEvent,
  LifecycleEventEvent,
  LoadEventFiredEvent,
  NavigatedWithinDocumentEvent,
  NavigateParams,
  NavigateResult,
  NavigateToHistoryEntryParams,
  PrintToPDFParams,
  PrintToPDFResult,
  ProduceCompilationCacheParams,
  ReloadParams,
  RemoveScriptToEvaluateOnLoadParams,
  RemoveScriptToEvaluateOnNewDocumentParams,
  ScreencastFrameAckParams,
  ScreencastFrameEvent,
  ScreencastVisibilityChangedEvent,
  SearchInResourceParams,
  SearchInResourceResult,
  SetAdBlockingEnabledParams,
  SetBypassCSPParams,
  SetDeviceMetricsOverrideParams,
  SetDeviceOrientationOverrideParams,
  SetDocumentContentParams,
  SetDownloadBehaviorParams,
  SetFontFamiliesParams,
  SetFontSizesParams,
  SetGeolocationOverrideParams,
  SetInterceptFileChooserDialogParams,
  SetLifecycleEventsEnabledParams,
  SetPrerenderingAllowedParams,
  SetRPHRegistrationModeParams,
  SetSPCTransactionModeParams,
  SetTouchEmulationEnabledParams,
  SetWebLifecycleStateParams,
  StartScreencastParams,
  StartScreenRecordingParams,
  StartScreenRecordingResult,
  StopScreenRecordingResult,
  WindowOpenEvent,
} from '../domains/page'

export interface PageApi {
  // ── Commands ──

  addScriptToEvaluateOnLoad(
    params: AddScriptToEvaluateOnLoadParams,
  ): Promise<AddScriptToEvaluateOnLoadResult>
  addScriptToEvaluateOnNewDocument(
    params: AddScriptToEvaluateOnNewDocumentParams,
  ): Promise<AddScriptToEvaluateOnNewDocumentResult>
  bringToFront(): Promise<void>
  captureScreenshot(
    params?: CaptureScreenshotParams,
  ): Promise<CaptureScreenshotResult>
  captureSnapshot(
    params?: CaptureSnapshotParams,
  ): Promise<CaptureSnapshotResult>
  clearDeviceMetricsOverride(): Promise<void>
  clearDeviceOrientationOverride(): Promise<void>
  clearGeolocationOverride(): Promise<void>
  createIsolatedWorld(
    params: CreateIsolatedWorldParams,
  ): Promise<CreateIsolatedWorldResult>
  deleteCookie(params: DeleteCookieParams): Promise<void>
  disable(): Promise<void>
  enable(params?: EnableParams): Promise<void>
  getAppManifest(params?: GetAppManifestParams): Promise<GetAppManifestResult>
  getInstallabilityErrors(): Promise<GetInstallabilityErrorsResult>
  getManifestIcons(): Promise<GetManifestIconsResult>
  getAppId(): Promise<GetAppIdResult>
  getAdScriptAncestry(
    params: GetAdScriptAncestryParams,
  ): Promise<GetAdScriptAncestryResult>
  getFrameTree(): Promise<GetFrameTreeResult>
  getLayoutMetrics(): Promise<GetLayoutMetricsResult>
  getNavigationHistory(): Promise<GetNavigationHistoryResult>
  resetNavigationHistory(): Promise<void>
  getResourceContent(
    params: GetResourceContentParams,
  ): Promise<GetResourceContentResult>
  getResourceTree(): Promise<GetResourceTreeResult>
  handleJavaScriptDialog(params: HandleJavaScriptDialogParams): Promise<void>
  navigate(params: NavigateParams): Promise<NavigateResult>
  navigateToHistoryEntry(params: NavigateToHistoryEntryParams): Promise<void>
  printToPDF(params?: PrintToPDFParams): Promise<PrintToPDFResult>
  reload(params?: ReloadParams): Promise<void>
  removeScriptToEvaluateOnLoad(
    params: RemoveScriptToEvaluateOnLoadParams,
  ): Promise<void>
  removeScriptToEvaluateOnNewDocument(
    params: RemoveScriptToEvaluateOnNewDocumentParams,
  ): Promise<void>
  screencastFrameAck(params: ScreencastFrameAckParams): Promise<void>
  searchInResource(
    params: SearchInResourceParams,
  ): Promise<SearchInResourceResult>
  setAdBlockingEnabled(params: SetAdBlockingEnabledParams): Promise<void>
  setBypassCSP(params: SetBypassCSPParams): Promise<void>
  getPermissionsPolicyState(
    params: GetPermissionsPolicyStateParams,
  ): Promise<GetPermissionsPolicyStateResult>
  getOriginTrials(params: GetOriginTrialsParams): Promise<GetOriginTrialsResult>
  setDeviceMetricsOverride(
    params: SetDeviceMetricsOverrideParams,
  ): Promise<void>
  setDeviceOrientationOverride(
    params: SetDeviceOrientationOverrideParams,
  ): Promise<void>
  setFontFamilies(params: SetFontFamiliesParams): Promise<void>
  setFontSizes(params: SetFontSizesParams): Promise<void>
  setDocumentContent(params: SetDocumentContentParams): Promise<void>
  setDownloadBehavior(params: SetDownloadBehaviorParams): Promise<void>
  setGeolocationOverride(params?: SetGeolocationOverrideParams): Promise<void>
  setLifecycleEventsEnabled(
    params: SetLifecycleEventsEnabledParams,
  ): Promise<void>
  setTouchEmulationEnabled(
    params: SetTouchEmulationEnabledParams,
  ): Promise<void>
  startScreencast(params?: StartScreencastParams): Promise<void>
  startScreenRecording(
    params?: StartScreenRecordingParams,
  ): Promise<StartScreenRecordingResult>
  stopScreenRecording(): Promise<StopScreenRecordingResult>
  stopLoading(): Promise<void>
  crash(): Promise<void>
  close(): Promise<void>
  setWebLifecycleState(params: SetWebLifecycleStateParams): Promise<void>
  stopScreencast(): Promise<void>
  produceCompilationCache(params: ProduceCompilationCacheParams): Promise<void>
  addCompilationCache(params: AddCompilationCacheParams): Promise<void>
  clearCompilationCache(): Promise<void>
  setSPCTransactionMode(params: SetSPCTransactionModeParams): Promise<void>
  setRPHRegistrationMode(params: SetRPHRegistrationModeParams): Promise<void>
  generateTestReport(params: GenerateTestReportParams): Promise<void>
  waitForDebugger(): Promise<void>
  setInterceptFileChooserDialog(
    params: SetInterceptFileChooserDialogParams,
  ): Promise<void>
  setPrerenderingAllowed(params: SetPrerenderingAllowedParams): Promise<void>
  getAnnotatedPageContent(
    params?: GetAnnotatedPageContentParams,
  ): Promise<GetAnnotatedPageContentResult>

  // ── Events ──

  on(
    event: 'domContentEventFired',
    handler: (params: DomContentEventFiredEvent) => void,
  ): () => void
  on(
    event: 'fileChooserOpened',
    handler: (params: FileChooserOpenedEvent) => void,
  ): () => void
  on(
    event: 'frameAttached',
    handler: (params: FrameAttachedEvent) => void,
  ): () => void
  on(
    event: 'frameClearedScheduledNavigation',
    handler: (params: FrameClearedScheduledNavigationEvent) => void,
  ): () => void
  on(
    event: 'frameDetached',
    handler: (params: FrameDetachedEvent) => void,
  ): () => void
  on(
    event: 'frameSubtreeWillBeDetached',
    handler: (params: FrameSubtreeWillBeDetachedEvent) => void,
  ): () => void
  on(
    event: 'frameNavigated',
    handler: (params: FrameNavigatedEvent) => void,
  ): () => void
  on(
    event: 'documentOpened',
    handler: (params: DocumentOpenedEvent) => void,
  ): () => void
  on(event: 'frameResized', handler: () => void): () => void
  on(
    event: 'frameStartedNavigating',
    handler: (params: FrameStartedNavigatingEvent) => void,
  ): () => void
  on(
    event: 'frameRequestedNavigation',
    handler: (params: FrameRequestedNavigationEvent) => void,
  ): () => void
  on(
    event: 'frameScheduledNavigation',
    handler: (params: FrameScheduledNavigationEvent) => void,
  ): () => void
  on(
    event: 'frameStartedLoading',
    handler: (params: FrameStartedLoadingEvent) => void,
  ): () => void
  on(
    event: 'frameStoppedLoading',
    handler: (params: FrameStoppedLoadingEvent) => void,
  ): () => void
  on(
    event: 'downloadWillBegin',
    handler: (params: DownloadWillBeginEvent) => void,
  ): () => void
  on(
    event: 'downloadProgress',
    handler: (params: DownloadProgressEvent) => void,
  ): () => void
  on(event: 'interstitialHidden', handler: () => void): () => void
  on(event: 'interstitialShown', handler: () => void): () => void
  on(
    event: 'javascriptDialogClosed',
    handler: (params: JavascriptDialogClosedEvent) => void,
  ): () => void
  on(
    event: 'javascriptDialogOpening',
    handler: (params: JavascriptDialogOpeningEvent) => void,
  ): () => void
  on(
    event: 'lifecycleEvent',
    handler: (params: LifecycleEventEvent) => void,
  ): () => void
  on(
    event: 'backForwardCacheNotUsed',
    handler: (params: BackForwardCacheNotUsedEvent) => void,
  ): () => void
  on(
    event: 'loadEventFired',
    handler: (params: LoadEventFiredEvent) => void,
  ): () => void
  on(
    event: 'navigatedWithinDocument',
    handler: (params: NavigatedWithinDocumentEvent) => void,
  ): () => void
  on(
    event: 'screencastFrame',
    handler: (params: ScreencastFrameEvent) => void,
  ): () => void
  on(
    event: 'screencastVisibilityChanged',
    handler: (params: ScreencastVisibilityChangedEvent) => void,
  ): () => void
  on(
    event: 'windowOpen',
    handler: (params: WindowOpenEvent) => void,
  ): () => void
  on(
    event: 'compilationCacheProduced',
    handler: (params: CompilationCacheProducedEvent) => void,
  ): () => void
}
