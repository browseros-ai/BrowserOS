// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  GetGridHighlightObjectsForTestParams,
  GetGridHighlightObjectsForTestResult,
  GetHighlightObjectForTestParams,
  GetHighlightObjectForTestResult,
  GetSourceOrderHighlightObjectForTestParams,
  GetSourceOrderHighlightObjectForTestResult,
  HighlightFrameParams,
  HighlightNodeParams,
  HighlightQuadParams,
  HighlightRectParams,
  HighlightSourceOrderParams,
  InspectedElementWindowRestoredEvent,
  InspectNodeRequestedEvent,
  InspectPanelShowRequestedEvent,
  NodeHighlightRequestedEvent,
  ScreenshotRequestedEvent,
  SetInspectModeParams,
  SetPausedInDebuggerMessageParams,
  SetShowAdHighlightsParams,
  SetShowContainerQueryOverlaysParams,
  SetShowDebugBordersParams,
  SetShowFlexOverlaysParams,
  SetShowFPSCounterParams,
  SetShowGridOverlaysParams,
  SetShowHingeParams,
  SetShowHitTestBordersParams,
  SetShowInspectedElementAnchorParams,
  SetShowIsolatedElementsParams,
  SetShowLayoutShiftRegionsParams,
  SetShowPaintRectsParams,
  SetShowScrollBottleneckRectsParams,
  SetShowScrollSnapOverlaysParams,
  SetShowViewportSizeOnResizeParams,
  SetShowWebVitalsParams,
  SetShowWindowControlsOverlayParams,
} from '../domains/overlay'

export interface OverlayApi {
  // ── Commands ──

  disable(): Promise<void>
  enable(): Promise<void>
  getHighlightObjectForTest(
    params: GetHighlightObjectForTestParams,
  ): Promise<GetHighlightObjectForTestResult>
  getGridHighlightObjectsForTest(
    params: GetGridHighlightObjectsForTestParams,
  ): Promise<GetGridHighlightObjectsForTestResult>
  getSourceOrderHighlightObjectForTest(
    params: GetSourceOrderHighlightObjectForTestParams,
  ): Promise<GetSourceOrderHighlightObjectForTestResult>
  hideHighlight(): Promise<void>
  highlightFrame(params: HighlightFrameParams): Promise<void>
  highlightNode(params: HighlightNodeParams): Promise<void>
  highlightQuad(params: HighlightQuadParams): Promise<void>
  highlightRect(params: HighlightRectParams): Promise<void>
  highlightSourceOrder(params: HighlightSourceOrderParams): Promise<void>
  setInspectMode(params: SetInspectModeParams): Promise<void>
  setShowAdHighlights(params: SetShowAdHighlightsParams): Promise<void>
  setPausedInDebuggerMessage(
    params?: SetPausedInDebuggerMessageParams,
  ): Promise<void>
  setShowDebugBorders(params: SetShowDebugBordersParams): Promise<void>
  setShowFPSCounter(params: SetShowFPSCounterParams): Promise<void>
  setShowGridOverlays(params: SetShowGridOverlaysParams): Promise<void>
  setShowFlexOverlays(params: SetShowFlexOverlaysParams): Promise<void>
  setShowScrollSnapOverlays(
    params: SetShowScrollSnapOverlaysParams,
  ): Promise<void>
  setShowContainerQueryOverlays(
    params: SetShowContainerQueryOverlaysParams,
  ): Promise<void>
  setShowInspectedElementAnchor(
    params: SetShowInspectedElementAnchorParams,
  ): Promise<void>
  setShowPaintRects(params: SetShowPaintRectsParams): Promise<void>
  setShowLayoutShiftRegions(
    params: SetShowLayoutShiftRegionsParams,
  ): Promise<void>
  setShowScrollBottleneckRects(
    params: SetShowScrollBottleneckRectsParams,
  ): Promise<void>
  setShowHitTestBorders(params: SetShowHitTestBordersParams): Promise<void>
  setShowWebVitals(params: SetShowWebVitalsParams): Promise<void>
  setShowViewportSizeOnResize(
    params: SetShowViewportSizeOnResizeParams,
  ): Promise<void>
  setShowHinge(params?: SetShowHingeParams): Promise<void>
  setShowIsolatedElements(params: SetShowIsolatedElementsParams): Promise<void>
  setShowWindowControlsOverlay(
    params?: SetShowWindowControlsOverlayParams,
  ): Promise<void>

  // ── Events ──

  on(
    event: 'inspectNodeRequested',
    handler: (params: InspectNodeRequestedEvent) => void,
  ): () => void
  on(
    event: 'nodeHighlightRequested',
    handler: (params: NodeHighlightRequestedEvent) => void,
  ): () => void
  on(
    event: 'screenshotRequested',
    handler: (params: ScreenshotRequestedEvent) => void,
  ): () => void
  on(
    event: 'inspectPanelShowRequested',
    handler: (params: InspectPanelShowRequestedEvent) => void,
  ): () => void
  on(
    event: 'inspectedElementWindowRestored',
    handler: (params: InspectedElementWindowRestoredEvent) => void,
  ): () => void
  on(event: 'inspectModeCanceled', handler: () => void): () => void
}
