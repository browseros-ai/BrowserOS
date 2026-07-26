// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { AdProvenance } from './network'
import type { FrameId } from './page'
import type {
  ExecutionContextId,
  RemoteObject,
  RemoteObjectId,
  StackTrace,
} from './runtime'

// ══ Types ══

export type NodeId = number

export type BackendNodeId = number

export type StyleSheetId = string

export interface BackendNode {
  nodeType: number
  nodeName: string
  backendNodeId: BackendNodeId
}

export type PseudoType =
  | 'first-line'
  | 'first-letter'
  | 'checkmark'
  | 'before'
  | 'after'
  | 'expand-icon'
  | 'picker-icon'
  | 'interest-hint'
  | 'marker'
  | 'backdrop'
  | 'column'
  | 'selection'
  | 'search-text'
  | 'target-text'
  | 'spelling-error'
  | 'grammar-error'
  | 'highlight'
  | 'first-line-inherited'
  | 'scroll-marker'
  | 'scroll-marker-group'
  | 'scroll-button'
  | 'scrollbar'
  | 'scrollbar-thumb'
  | 'scrollbar-button'
  | 'scrollbar-track'
  | 'scrollbar-track-piece'
  | 'scrollbar-corner'
  | 'resizer'
  | 'input-list-button'
  | 'view-transition'
  | 'view-transition-group'
  | 'view-transition-image-pair'
  | 'view-transition-group-children'
  | 'view-transition-old'
  | 'view-transition-new'
  | 'placeholder'
  | 'file-selector-button'
  | 'details-content'
  | 'picker'
  | 'permission-icon'
  | 'overscroll-area-parent'

export type ShadowRootType = 'user-agent' | 'open' | 'closed'

export type CompatibilityMode =
  | 'QuirksMode'
  | 'LimitedQuirksMode'
  | 'NoQuirksMode'

export type PhysicalAxes = 'Horizontal' | 'Vertical' | 'Both'

export type LogicalAxes = 'Inline' | 'Block' | 'Both'

export type ScrollOrientation = 'horizontal' | 'vertical'

export interface Node {
  nodeId: NodeId
  parentId?: NodeId
  backendNodeId: BackendNodeId
  nodeType: number
  nodeName: string
  localName: string
  nodeValue: string
  childNodeCount?: number
  children?: Node[]
  attributes?: string[]
  documentURL?: string
  baseURL?: string
  publicId?: string
  systemId?: string
  internalSubset?: string
  xmlVersion?: string
  name?: string
  value?: string
  pseudoType?: PseudoType
  pseudoIdentifier?: string
  shadowRootType?: ShadowRootType
  frameId?: FrameId
  contentDocument?: Node
  shadowRoots?: Node[]
  templateContent?: Node
  pseudoElements?: Node[]
  importedDocument?: Node
  distributedNodes?: BackendNode[]
  isSVG?: boolean
  compatibilityMode?: CompatibilityMode
  assignedSlot?: BackendNode
  isScrollable?: boolean
  affectedByStartingStyles?: boolean
  adoptedStyleSheets?: StyleSheetId[]
  adProvenance?: AdProvenance
}

export interface DetachedElementInfo {
  treeNode: Node
  retainedNodeIds: NodeId[]
}

export interface RGBA {
  r: number
  g: number
  b: number
  a?: number
}

export type Quad = number[]

export interface BoxModel {
  content: Quad
  padding: Quad
  border: Quad
  margin: Quad
  width: number
  height: number
  shapeOutside?: ShapeOutsideInfo
}

export interface ShapeOutsideInfo {
  bounds: Quad
  shape: unknown[]
  marginShape: unknown[]
}

export interface Rect {
  x: number
  y: number
  width: number
  height: number
}

export interface CSSComputedStyleProperty {
  name: string
  value: string
}

// ══ Commands ══

export interface CollectClassNamesFromSubtreeParams {
  nodeId: NodeId
}

export interface CollectClassNamesFromSubtreeResult {
  classNames: string[]
}

export interface CopyToParams {
  nodeId: NodeId
  targetNodeId: NodeId
  insertBeforeNodeId?: NodeId
}

export interface CopyToResult {
  nodeId: NodeId
}

export interface DescribeNodeParams {
  nodeId?: NodeId
  backendNodeId?: BackendNodeId
  objectId?: RemoteObjectId
  depth?: number
  pierce?: boolean
}

export interface DescribeNodeResult {
  node: Node
}

export interface ScrollIntoViewIfNeededParams {
  nodeId?: NodeId
  backendNodeId?: BackendNodeId
  objectId?: RemoteObjectId
  rect?: Rect
}

export interface DiscardSearchResultsParams {
  searchId: string
}

export interface EnableParams {
  includeWhitespace?: 'none' | 'all'
}

export interface FocusParams {
  nodeId?: NodeId
  backendNodeId?: BackendNodeId
  objectId?: RemoteObjectId
}

export interface GetAttributesParams {
  nodeId: NodeId
}

export interface GetAttributesResult {
  attributes: string[]
}

export interface GetBoxModelParams {
  nodeId?: NodeId
  backendNodeId?: BackendNodeId
  objectId?: RemoteObjectId
}

export interface GetBoxModelResult {
  model: BoxModel
}

export interface GetContentQuadsParams {
  nodeId?: NodeId
  backendNodeId?: BackendNodeId
  objectId?: RemoteObjectId
}

export interface GetContentQuadsResult {
  quads: Quad[]
}

export interface GetDocumentParams {
  depth?: number
  pierce?: boolean
}

export interface GetDocumentResult {
  root: Node
}

export interface GetFlattenedDocumentParams {
  depth?: number
  pierce?: boolean
}

export interface GetFlattenedDocumentResult {
  nodes: Node[]
}

export interface GetNodesForSubtreeByStyleParams {
  nodeId: NodeId
  computedStyles: CSSComputedStyleProperty[]
  pierce?: boolean
}

export interface GetNodesForSubtreeByStyleResult {
  nodeIds: NodeId[]
}

export interface GetNodeForLocationParams {
  x: number
  y: number
  includeUserAgentShadowDOM?: boolean
  ignorePointerEventsNone?: boolean
}

export interface GetNodeForLocationResult {
  backendNodeId: BackendNodeId
  frameId: FrameId
  nodeId?: NodeId
}

export interface GetOuterHTMLParams {
  nodeId?: NodeId
  backendNodeId?: BackendNodeId
  objectId?: RemoteObjectId
  includeShadowDOM?: boolean
}

export interface GetOuterHTMLResult {
  outerHTML: string
}

export interface GetRelayoutBoundaryParams {
  nodeId: NodeId
}

export interface GetRelayoutBoundaryResult {
  nodeId: NodeId
}

export interface GetSearchResultsParams {
  searchId: string
  fromIndex: number
  toIndex: number
}

export interface GetSearchResultsResult {
  nodeIds: NodeId[]
}

export interface MoveToParams {
  nodeId: NodeId
  targetNodeId: NodeId
  insertBeforeNodeId?: NodeId
}

export interface MoveToResult {
  nodeId: NodeId
}

export interface PerformSearchParams {
  query: string
  includeUserAgentShadowDOM?: boolean
}

export interface PerformSearchResult {
  searchId: string
  resultCount: number
}

export interface PushNodeByPathToFrontendParams {
  path: string
}

export interface PushNodeByPathToFrontendResult {
  nodeId: NodeId
}

export interface PushNodesByBackendIdsToFrontendParams {
  backendNodeIds: BackendNodeId[]
}

export interface PushNodesByBackendIdsToFrontendResult {
  nodeIds: NodeId[]
}

export interface QuerySelectorParams {
  nodeId: NodeId
  selector: string
}

export interface QuerySelectorResult {
  nodeId: NodeId
}

export interface QuerySelectorAllParams {
  nodeId: NodeId
  selector: string
}

export interface QuerySelectorAllResult {
  nodeIds: NodeId[]
}

export interface GetTopLayerElementsResult {
  nodeIds: NodeId[]
}

export interface GetElementByRelationParams {
  nodeId: NodeId
  relation: 'PopoverTarget' | 'InterestTarget' | 'CommandFor'
}

export interface GetElementByRelationResult {
  nodeId: NodeId
}

export interface RemoveAttributeParams {
  nodeId: NodeId
  name: string
}

export interface RemoveNodeParams {
  nodeId: NodeId
}

export interface RequestChildNodesParams {
  nodeId: NodeId
  depth?: number
  pierce?: boolean
}

export interface RequestNodeParams {
  objectId: RemoteObjectId
}

export interface RequestNodeResult {
  nodeId: NodeId
}

export interface ResolveNodeParams {
  nodeId?: NodeId
  backendNodeId?: BackendNodeId
  objectGroup?: string
  executionContextId?: ExecutionContextId
}

export interface ResolveNodeResult {
  object: RemoteObject
}

export interface SetAttributeValueParams {
  nodeId: NodeId
  name: string
  value: string
}

export interface SetAttributesAsTextParams {
  nodeId: NodeId
  text: string
  name?: string
}

export interface SetFileInputFilesParams {
  files: string[]
  nodeId?: NodeId
  backendNodeId?: BackendNodeId
  objectId?: RemoteObjectId
}

export interface SetNodeStackTracesEnabledParams {
  enable: boolean
}

export interface GetNodeStackTracesParams {
  nodeId: NodeId
}

export interface GetNodeStackTracesResult {
  creation?: StackTrace
}

export interface GetFileInfoParams {
  objectId: RemoteObjectId
}

export interface GetFileInfoResult {
  path: string
}

export interface GetDetachedDomNodesResult {
  detachedNodes: DetachedElementInfo[]
}

export interface SetInspectedNodeParams {
  nodeId: NodeId
}

export interface SetNodeNameParams {
  nodeId: NodeId
  name: string
}

export interface SetNodeNameResult {
  nodeId: NodeId
}

export interface SetNodeValueParams {
  nodeId: NodeId
  value: string
}

export interface SetOuterHTMLParams {
  nodeId: NodeId
  outerHTML: string
}

export interface GetFrameOwnerParams {
  frameId: FrameId
}

export interface GetFrameOwnerResult {
  backendNodeId: BackendNodeId
  nodeId?: NodeId
}

export interface GetContainerForNodeParams {
  nodeId: NodeId
  containerName?: string
  physicalAxes?: PhysicalAxes
  logicalAxes?: LogicalAxes
  queriesScrollState?: boolean
  queriesAnchored?: boolean
}

export interface GetContainerForNodeResult {
  nodeId?: NodeId
}

export interface GetQueryingDescendantsForContainerParams {
  nodeId: NodeId
}

export interface GetQueryingDescendantsForContainerResult {
  nodeIds: NodeId[]
}

export interface GetAnchorElementParams {
  nodeId: NodeId
  anchorSpecifier?: string
}

export interface GetAnchorElementResult {
  nodeId: NodeId
}

export interface ForceShowPopoverParams {
  nodeId: NodeId
  enable: boolean
}

export interface ForceShowPopoverResult {
  nodeIds: NodeId[]
}

// ══ Events ══

export interface AttributeModifiedEvent {
  nodeId: NodeId
  name: string
  value: string
}

export interface AdoptedStyleSheetsModifiedEvent {
  nodeId: NodeId
  adoptedStyleSheets: StyleSheetId[]
}

export interface AttributeRemovedEvent {
  nodeId: NodeId
  name: string
}

export interface CharacterDataModifiedEvent {
  nodeId: NodeId
  characterData: string
}

export interface ChildNodeCountUpdatedEvent {
  nodeId: NodeId
  childNodeCount: number
}

export interface ChildNodeInsertedEvent {
  parentNodeId: NodeId
  previousNodeId: NodeId
  node: Node
}

export interface ChildNodeRemovedEvent {
  parentNodeId: NodeId
  nodeId: NodeId
}

export interface DistributedNodesUpdatedEvent {
  insertionPointId: NodeId
  distributedNodes: BackendNode[]
}

export interface InlineStyleInvalidatedEvent {
  nodeIds: NodeId[]
}

export interface PseudoElementAddedEvent {
  parentId: NodeId
  pseudoElement: Node
}

export interface ScrollableFlagUpdatedEvent {
  nodeId: NodeId
  isScrollable: boolean
}

export interface AdRelatedStateUpdatedEvent {
  nodeId: NodeId
  adProvenance?: AdProvenance
}

export interface AffectedByStartingStylesFlagUpdatedEvent {
  nodeId: NodeId
  affectedByStartingStyles: boolean
}

export interface PseudoElementRemovedEvent {
  parentId: NodeId
  pseudoElementId: NodeId
}

export interface SetChildNodesEvent {
  parentId: NodeId
  nodes: Node[]
}

export interface ShadowRootPoppedEvent {
  hostId: NodeId
  rootId: NodeId
}

export interface ShadowRootPushedEvent {
  hostId: NodeId
  root: Node
}
