// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { BackendNodeId, NodeId, Quad, Rect, RGBA } from './dom'
import type { FrameId, Viewport } from './page'
import type { RemoteObjectId } from './runtime'

// ══ Types ══

export interface SourceOrderConfig {
  parentOutlineColor: RGBA
  childOutlineColor: RGBA
}

export interface GridHighlightConfig {
  showGridExtensionLines?: boolean
  showPositiveLineNumbers?: boolean
  showNegativeLineNumbers?: boolean
  showAreaNames?: boolean
  showLineNames?: boolean
  showTrackSizes?: boolean
  gridBorderColor?: RGBA
  cellBorderColor?: RGBA
  rowLineColor?: RGBA
  columnLineColor?: RGBA
  gridBorderDash?: boolean
  cellBorderDash?: boolean
  rowLineDash?: boolean
  columnLineDash?: boolean
  rowGapColor?: RGBA
  rowHatchColor?: RGBA
  columnGapColor?: RGBA
  columnHatchColor?: RGBA
  areaBorderColor?: RGBA
  gridBackgroundColor?: RGBA
}

export interface FlexContainerHighlightConfig {
  containerBorder?: LineStyle
  lineSeparator?: LineStyle
  itemSeparator?: LineStyle
  mainDistributedSpace?: BoxStyle
  crossDistributedSpace?: BoxStyle
  rowGapSpace?: BoxStyle
  columnGapSpace?: BoxStyle
  crossAlignment?: LineStyle
}

export interface FlexItemHighlightConfig {
  baseSizeBox?: BoxStyle
  baseSizeBorder?: LineStyle
  flexibilityArrow?: LineStyle
}

export interface LineStyle {
  color?: RGBA
  pattern?: 'dashed' | 'dotted'
}

export interface BoxStyle {
  fillColor?: RGBA
  hatchColor?: RGBA
}

export type ContrastAlgorithm = 'aa' | 'aaa' | 'apca'

export interface HighlightConfig {
  showInfo?: boolean
  showStyles?: boolean
  showRulers?: boolean
  showAccessibilityInfo?: boolean
  showExtensionLines?: boolean
  contentColor?: RGBA
  paddingColor?: RGBA
  borderColor?: RGBA
  marginColor?: RGBA
  eventTargetColor?: RGBA
  shapeColor?: RGBA
  shapeMarginColor?: RGBA
  cssGridColor?: RGBA
  colorFormat?: ColorFormat
  gridHighlightConfig?: GridHighlightConfig
  flexContainerHighlightConfig?: FlexContainerHighlightConfig
  flexItemHighlightConfig?: FlexItemHighlightConfig
  contrastAlgorithm?: ContrastAlgorithm
  containerQueryContainerHighlightConfig?: ContainerQueryContainerHighlightConfig
}

export type ColorFormat = 'rgb' | 'hsl' | 'hwb' | 'hex'

export interface GridNodeHighlightConfig {
  gridHighlightConfig: GridHighlightConfig
  nodeId: NodeId
}

export interface FlexNodeHighlightConfig {
  flexContainerHighlightConfig: FlexContainerHighlightConfig
  nodeId: NodeId
}

export interface ScrollSnapContainerHighlightConfig {
  snapportBorder?: LineStyle
  snapAreaBorder?: LineStyle
  scrollMarginColor?: RGBA
  scrollPaddingColor?: RGBA
}

export interface ScrollSnapHighlightConfig {
  scrollSnapContainerHighlightConfig: ScrollSnapContainerHighlightConfig
  nodeId: NodeId
}

export interface HingeConfig {
  rect: Rect
  contentColor?: RGBA
  outlineColor?: RGBA
}

export interface WindowControlsOverlayConfig {
  showCSS: boolean
  selectedPlatform: string
  themeColor: string
}

export interface ContainerQueryHighlightConfig {
  containerQueryContainerHighlightConfig: ContainerQueryContainerHighlightConfig
  nodeId: NodeId
}

export interface ContainerQueryContainerHighlightConfig {
  containerBorder?: LineStyle
  descendantBorder?: LineStyle
}

export interface IsolatedElementHighlightConfig {
  isolationModeHighlightConfig: IsolationModeHighlightConfig
  nodeId: NodeId
}

export interface IsolationModeHighlightConfig {
  resizerColor?: RGBA
  resizerHandleColor?: RGBA
  maskColor?: RGBA
}

export type InspectMode =
  | 'searchForNode'
  | 'searchForUAShadowDOM'
  | 'captureAreaScreenshot'
  | 'none'

export interface InspectedElementAnchorConfig {
  nodeId?: NodeId
  backendNodeId?: BackendNodeId
}

// ══ Commands ══

export interface GetHighlightObjectForTestParams {
  nodeId: NodeId
  includeDistance?: boolean
  includeStyle?: boolean
  colorFormat?: ColorFormat
  showAccessibilityInfo?: boolean
}

export interface GetHighlightObjectForTestResult {
  highlight: Record<string, unknown>
}

export interface GetGridHighlightObjectsForTestParams {
  nodeIds: NodeId[]
}

export interface GetGridHighlightObjectsForTestResult {
  highlights: Record<string, unknown>
}

export interface GetSourceOrderHighlightObjectForTestParams {
  nodeId: NodeId
}

export interface GetSourceOrderHighlightObjectForTestResult {
  highlight: Record<string, unknown>
}

export interface HighlightFrameParams {
  frameId: FrameId
  contentColor?: RGBA
  contentOutlineColor?: RGBA
}

export interface HighlightNodeParams {
  highlightConfig: HighlightConfig
  nodeId?: NodeId
  backendNodeId?: BackendNodeId
  objectId?: RemoteObjectId
  selector?: string
}

export interface HighlightQuadParams {
  quad: Quad
  color?: RGBA
  outlineColor?: RGBA
}

export interface HighlightRectParams {
  x: number
  y: number
  width: number
  height: number
  color?: RGBA
  outlineColor?: RGBA
}

export interface HighlightSourceOrderParams {
  sourceOrderConfig: SourceOrderConfig
  nodeId?: NodeId
  backendNodeId?: BackendNodeId
  objectId?: RemoteObjectId
}

export interface SetInspectModeParams {
  mode: InspectMode
  highlightConfig?: HighlightConfig
}

export interface SetShowAdHighlightsParams {
  show: boolean
}

export interface SetPausedInDebuggerMessageParams {
  message?: string
}

export interface SetShowDebugBordersParams {
  show: boolean
}

export interface SetShowFPSCounterParams {
  show: boolean
}

export interface SetShowGridOverlaysParams {
  gridNodeHighlightConfigs: GridNodeHighlightConfig[]
}

export interface SetShowFlexOverlaysParams {
  flexNodeHighlightConfigs: FlexNodeHighlightConfig[]
}

export interface SetShowScrollSnapOverlaysParams {
  scrollSnapHighlightConfigs: ScrollSnapHighlightConfig[]
}

export interface SetShowContainerQueryOverlaysParams {
  containerQueryHighlightConfigs: ContainerQueryHighlightConfig[]
}

export interface SetShowInspectedElementAnchorParams {
  inspectedElementAnchorConfig: InspectedElementAnchorConfig
}

export interface SetShowPaintRectsParams {
  result: boolean
}

export interface SetShowLayoutShiftRegionsParams {
  result: boolean
}

export interface SetShowScrollBottleneckRectsParams {
  show: boolean
}

export interface SetShowHitTestBordersParams {
  show: boolean
}

export interface SetShowWebVitalsParams {
  show: boolean
}

export interface SetShowViewportSizeOnResizeParams {
  show: boolean
}

export interface SetShowHingeParams {
  hingeConfig?: HingeConfig
}

export interface SetShowIsolatedElementsParams {
  isolatedElementHighlightConfigs: IsolatedElementHighlightConfig[]
}

export interface SetShowWindowControlsOverlayParams {
  windowControlsOverlayConfig?: WindowControlsOverlayConfig
}

// ══ Events ══

export interface InspectNodeRequestedEvent {
  backendNodeId: BackendNodeId
}

export interface NodeHighlightRequestedEvent {
  nodeId: NodeId
}

export interface ScreenshotRequestedEvent {
  viewport: Viewport
}

export interface InspectPanelShowRequestedEvent {
  backendNodeId: BackendNodeId
}

export interface InspectedElementWindowRestoredEvent {
  backendNodeId: BackendNodeId
}
