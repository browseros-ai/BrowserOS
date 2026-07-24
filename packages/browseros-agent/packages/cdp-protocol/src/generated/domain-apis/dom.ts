// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  AdoptedStyleSheetsModifiedEvent,
  AdRelatedStateUpdatedEvent,
  AffectedByStartingStylesFlagUpdatedEvent,
  AttributeModifiedEvent,
  AttributeRemovedEvent,
  CharacterDataModifiedEvent,
  ChildNodeCountUpdatedEvent,
  ChildNodeInsertedEvent,
  ChildNodeRemovedEvent,
  CollectClassNamesFromSubtreeParams,
  CollectClassNamesFromSubtreeResult,
  CopyToParams,
  CopyToResult,
  DescribeNodeParams,
  DescribeNodeResult,
  DiscardSearchResultsParams,
  DistributedNodesUpdatedEvent,
  EnableParams,
  FocusParams,
  ForceShowPopoverParams,
  ForceShowPopoverResult,
  GetAnchorElementParams,
  GetAnchorElementResult,
  GetAttributesParams,
  GetAttributesResult,
  GetBoxModelParams,
  GetBoxModelResult,
  GetContainerForNodeParams,
  GetContainerForNodeResult,
  GetContentQuadsParams,
  GetContentQuadsResult,
  GetDetachedDomNodesResult,
  GetDocumentParams,
  GetDocumentResult,
  GetElementByRelationParams,
  GetElementByRelationResult,
  GetFileInfoParams,
  GetFileInfoResult,
  GetFlattenedDocumentParams,
  GetFlattenedDocumentResult,
  GetFrameOwnerParams,
  GetFrameOwnerResult,
  GetNodeForLocationParams,
  GetNodeForLocationResult,
  GetNodeStackTracesParams,
  GetNodeStackTracesResult,
  GetNodesForSubtreeByStyleParams,
  GetNodesForSubtreeByStyleResult,
  GetOuterHTMLParams,
  GetOuterHTMLResult,
  GetQueryingDescendantsForContainerParams,
  GetQueryingDescendantsForContainerResult,
  GetRelayoutBoundaryParams,
  GetRelayoutBoundaryResult,
  GetSearchResultsParams,
  GetSearchResultsResult,
  GetTopLayerElementsResult,
  InlineStyleInvalidatedEvent,
  MoveToParams,
  MoveToResult,
  PerformSearchParams,
  PerformSearchResult,
  PseudoElementAddedEvent,
  PseudoElementRemovedEvent,
  PushNodeByPathToFrontendParams,
  PushNodeByPathToFrontendResult,
  PushNodesByBackendIdsToFrontendParams,
  PushNodesByBackendIdsToFrontendResult,
  QuerySelectorAllParams,
  QuerySelectorAllResult,
  QuerySelectorParams,
  QuerySelectorResult,
  RemoveAttributeParams,
  RemoveNodeParams,
  RequestChildNodesParams,
  RequestNodeParams,
  RequestNodeResult,
  ResolveNodeParams,
  ResolveNodeResult,
  ScrollableFlagUpdatedEvent,
  ScrollIntoViewIfNeededParams,
  SetAttributesAsTextParams,
  SetAttributeValueParams,
  SetChildNodesEvent,
  SetFileInputFilesParams,
  SetInspectedNodeParams,
  SetNodeNameParams,
  SetNodeNameResult,
  SetNodeStackTracesEnabledParams,
  SetNodeValueParams,
  SetOuterHTMLParams,
  ShadowRootPoppedEvent,
  ShadowRootPushedEvent,
} from '../domains/dom'

export interface DOMApi {
  // ── Commands ──

  collectClassNamesFromSubtree(
    params: CollectClassNamesFromSubtreeParams,
  ): Promise<CollectClassNamesFromSubtreeResult>
  copyTo(params: CopyToParams): Promise<CopyToResult>
  describeNode(params?: DescribeNodeParams): Promise<DescribeNodeResult>
  scrollIntoViewIfNeeded(params?: ScrollIntoViewIfNeededParams): Promise<void>
  disable(): Promise<void>
  discardSearchResults(params: DiscardSearchResultsParams): Promise<void>
  enable(params?: EnableParams): Promise<void>
  focus(params?: FocusParams): Promise<void>
  getAttributes(params: GetAttributesParams): Promise<GetAttributesResult>
  getBoxModel(params?: GetBoxModelParams): Promise<GetBoxModelResult>
  getContentQuads(
    params?: GetContentQuadsParams,
  ): Promise<GetContentQuadsResult>
  getDocument(params?: GetDocumentParams): Promise<GetDocumentResult>
  getFlattenedDocument(
    params?: GetFlattenedDocumentParams,
  ): Promise<GetFlattenedDocumentResult>
  getNodesForSubtreeByStyle(
    params: GetNodesForSubtreeByStyleParams,
  ): Promise<GetNodesForSubtreeByStyleResult>
  getNodeForLocation(
    params: GetNodeForLocationParams,
  ): Promise<GetNodeForLocationResult>
  getOuterHTML(params?: GetOuterHTMLParams): Promise<GetOuterHTMLResult>
  getRelayoutBoundary(
    params: GetRelayoutBoundaryParams,
  ): Promise<GetRelayoutBoundaryResult>
  getSearchResults(
    params: GetSearchResultsParams,
  ): Promise<GetSearchResultsResult>
  hideHighlight(): Promise<void>
  highlightNode(): Promise<void>
  highlightRect(): Promise<void>
  markUndoableState(): Promise<void>
  moveTo(params: MoveToParams): Promise<MoveToResult>
  performSearch(params: PerformSearchParams): Promise<PerformSearchResult>
  pushNodeByPathToFrontend(
    params: PushNodeByPathToFrontendParams,
  ): Promise<PushNodeByPathToFrontendResult>
  pushNodesByBackendIdsToFrontend(
    params: PushNodesByBackendIdsToFrontendParams,
  ): Promise<PushNodesByBackendIdsToFrontendResult>
  querySelector(params: QuerySelectorParams): Promise<QuerySelectorResult>
  querySelectorAll(
    params: QuerySelectorAllParams,
  ): Promise<QuerySelectorAllResult>
  getTopLayerElements(): Promise<GetTopLayerElementsResult>
  getElementByRelation(
    params: GetElementByRelationParams,
  ): Promise<GetElementByRelationResult>
  redo(): Promise<void>
  removeAttribute(params: RemoveAttributeParams): Promise<void>
  removeNode(params: RemoveNodeParams): Promise<void>
  requestChildNodes(params: RequestChildNodesParams): Promise<void>
  requestNode(params: RequestNodeParams): Promise<RequestNodeResult>
  resolveNode(params?: ResolveNodeParams): Promise<ResolveNodeResult>
  setAttributeValue(params: SetAttributeValueParams): Promise<void>
  setAttributesAsText(params: SetAttributesAsTextParams): Promise<void>
  setFileInputFiles(params: SetFileInputFilesParams): Promise<void>
  setNodeStackTracesEnabled(
    params: SetNodeStackTracesEnabledParams,
  ): Promise<void>
  getNodeStackTraces(
    params: GetNodeStackTracesParams,
  ): Promise<GetNodeStackTracesResult>
  getFileInfo(params: GetFileInfoParams): Promise<GetFileInfoResult>
  getDetachedDomNodes(): Promise<GetDetachedDomNodesResult>
  setInspectedNode(params: SetInspectedNodeParams): Promise<void>
  setNodeName(params: SetNodeNameParams): Promise<SetNodeNameResult>
  setNodeValue(params: SetNodeValueParams): Promise<void>
  setOuterHTML(params: SetOuterHTMLParams): Promise<void>
  undo(): Promise<void>
  getFrameOwner(params: GetFrameOwnerParams): Promise<GetFrameOwnerResult>
  getContainerForNode(
    params: GetContainerForNodeParams,
  ): Promise<GetContainerForNodeResult>
  getQueryingDescendantsForContainer(
    params: GetQueryingDescendantsForContainerParams,
  ): Promise<GetQueryingDescendantsForContainerResult>
  getAnchorElement(
    params: GetAnchorElementParams,
  ): Promise<GetAnchorElementResult>
  forceShowPopover(
    params: ForceShowPopoverParams,
  ): Promise<ForceShowPopoverResult>

  // ── Events ──

  on(
    event: 'attributeModified',
    handler: (params: AttributeModifiedEvent) => void,
  ): () => void
  on(
    event: 'adoptedStyleSheetsModified',
    handler: (params: AdoptedStyleSheetsModifiedEvent) => void,
  ): () => void
  on(
    event: 'attributeRemoved',
    handler: (params: AttributeRemovedEvent) => void,
  ): () => void
  on(
    event: 'characterDataModified',
    handler: (params: CharacterDataModifiedEvent) => void,
  ): () => void
  on(
    event: 'childNodeCountUpdated',
    handler: (params: ChildNodeCountUpdatedEvent) => void,
  ): () => void
  on(
    event: 'childNodeInserted',
    handler: (params: ChildNodeInsertedEvent) => void,
  ): () => void
  on(
    event: 'childNodeRemoved',
    handler: (params: ChildNodeRemovedEvent) => void,
  ): () => void
  on(
    event: 'distributedNodesUpdated',
    handler: (params: DistributedNodesUpdatedEvent) => void,
  ): () => void
  on(event: 'documentUpdated', handler: () => void): () => void
  on(
    event: 'inlineStyleInvalidated',
    handler: (params: InlineStyleInvalidatedEvent) => void,
  ): () => void
  on(
    event: 'pseudoElementAdded',
    handler: (params: PseudoElementAddedEvent) => void,
  ): () => void
  on(event: 'topLayerElementsUpdated', handler: () => void): () => void
  on(
    event: 'scrollableFlagUpdated',
    handler: (params: ScrollableFlagUpdatedEvent) => void,
  ): () => void
  on(
    event: 'adRelatedStateUpdated',
    handler: (params: AdRelatedStateUpdatedEvent) => void,
  ): () => void
  on(
    event: 'affectedByStartingStylesFlagUpdated',
    handler: (params: AffectedByStartingStylesFlagUpdatedEvent) => void,
  ): () => void
  on(
    event: 'pseudoElementRemoved',
    handler: (params: PseudoElementRemovedEvent) => void,
  ): () => void
  on(
    event: 'setChildNodes',
    handler: (params: SetChildNodesEvent) => void,
  ): () => void
  on(
    event: 'shadowRootPopped',
    handler: (params: ShadowRootPoppedEvent) => void,
  ): () => void
  on(
    event: 'shadowRootPushed',
    handler: (params: ShadowRootPushedEvent) => void,
  ): () => void
}
