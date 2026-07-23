// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  AddRuleParams,
  AddRuleResult,
  CollectClassNamesParams,
  CollectClassNamesResult,
  ComputedStyleUpdatedEvent,
  CreateStyleSheetParams,
  CreateStyleSheetResult,
  FontsUpdatedEvent,
  ForcePseudoStateParams,
  ForceStartingStyleParams,
  GetAnimatedStylesForNodeParams,
  GetAnimatedStylesForNodeResult,
  GetBackgroundColorsParams,
  GetBackgroundColorsResult,
  GetComputedStyleForNodeParams,
  GetComputedStyleForNodeResult,
  GetEnvironmentVariablesResult,
  GetInlineStylesForNodeParams,
  GetInlineStylesForNodeResult,
  GetLayersForNodeParams,
  GetLayersForNodeResult,
  GetLocationForSelectorParams,
  GetLocationForSelectorResult,
  GetLonghandPropertiesParams,
  GetLonghandPropertiesResult,
  GetMatchedStylesForNodeParams,
  GetMatchedStylesForNodeResult,
  GetMediaQueriesResult,
  GetPlatformFontsForNodeParams,
  GetPlatformFontsForNodeResult,
  GetStyleSheetTextParams,
  GetStyleSheetTextResult,
  ResolveValuesParams,
  ResolveValuesResult,
  SetContainerQueryTextParams,
  SetContainerQueryTextResult,
  SetEffectivePropertyValueForNodeParams,
  SetKeyframeKeyParams,
  SetKeyframeKeyResult,
  SetLocalFontsEnabledParams,
  SetMediaTextParams,
  SetMediaTextResult,
  SetNavigationTextParams,
  SetNavigationTextResult,
  SetPropertyRulePropertyNameParams,
  SetPropertyRulePropertyNameResult,
  SetRuleSelectorParams,
  SetRuleSelectorResult,
  SetScopeTextParams,
  SetScopeTextResult,
  SetStyleSheetTextParams,
  SetStyleSheetTextResult,
  SetStyleTextsParams,
  SetStyleTextsResult,
  SetSupportsTextParams,
  SetSupportsTextResult,
  StopRuleUsageTrackingResult,
  StyleSheetAddedEvent,
  StyleSheetChangedEvent,
  StyleSheetRemovedEvent,
  TakeComputedStyleUpdatesResult,
  TakeCoverageDeltaResult,
  TrackComputedStyleUpdatesForNodeParams,
  TrackComputedStyleUpdatesParams,
} from '../domains/css'

export interface CSSApi {
  // ── Commands ──

  addRule(params: AddRuleParams): Promise<AddRuleResult>
  collectClassNames(
    params: CollectClassNamesParams,
  ): Promise<CollectClassNamesResult>
  createStyleSheet(
    params: CreateStyleSheetParams,
  ): Promise<CreateStyleSheetResult>
  disable(): Promise<void>
  enable(): Promise<void>
  forcePseudoState(params: ForcePseudoStateParams): Promise<void>
  forceStartingStyle(params: ForceStartingStyleParams): Promise<void>
  getBackgroundColors(
    params: GetBackgroundColorsParams,
  ): Promise<GetBackgroundColorsResult>
  getComputedStyleForNode(
    params: GetComputedStyleForNodeParams,
  ): Promise<GetComputedStyleForNodeResult>
  resolveValues(params: ResolveValuesParams): Promise<ResolveValuesResult>
  getLonghandProperties(
    params: GetLonghandPropertiesParams,
  ): Promise<GetLonghandPropertiesResult>
  getInlineStylesForNode(
    params: GetInlineStylesForNodeParams,
  ): Promise<GetInlineStylesForNodeResult>
  getAnimatedStylesForNode(
    params: GetAnimatedStylesForNodeParams,
  ): Promise<GetAnimatedStylesForNodeResult>
  getMatchedStylesForNode(
    params: GetMatchedStylesForNodeParams,
  ): Promise<GetMatchedStylesForNodeResult>
  getEnvironmentVariables(): Promise<GetEnvironmentVariablesResult>
  getMediaQueries(): Promise<GetMediaQueriesResult>
  getPlatformFontsForNode(
    params: GetPlatformFontsForNodeParams,
  ): Promise<GetPlatformFontsForNodeResult>
  getStyleSheetText(
    params: GetStyleSheetTextParams,
  ): Promise<GetStyleSheetTextResult>
  getLayersForNode(
    params: GetLayersForNodeParams,
  ): Promise<GetLayersForNodeResult>
  getLocationForSelector(
    params: GetLocationForSelectorParams,
  ): Promise<GetLocationForSelectorResult>
  trackComputedStyleUpdatesForNode(
    params?: TrackComputedStyleUpdatesForNodeParams,
  ): Promise<void>
  trackComputedStyleUpdates(
    params: TrackComputedStyleUpdatesParams,
  ): Promise<void>
  takeComputedStyleUpdates(): Promise<TakeComputedStyleUpdatesResult>
  setEffectivePropertyValueForNode(
    params: SetEffectivePropertyValueForNodeParams,
  ): Promise<void>
  setPropertyRulePropertyName(
    params: SetPropertyRulePropertyNameParams,
  ): Promise<SetPropertyRulePropertyNameResult>
  setKeyframeKey(params: SetKeyframeKeyParams): Promise<SetKeyframeKeyResult>
  setMediaText(params: SetMediaTextParams): Promise<SetMediaTextResult>
  setContainerQueryText(
    params: SetContainerQueryTextParams,
  ): Promise<SetContainerQueryTextResult>
  setSupportsText(params: SetSupportsTextParams): Promise<SetSupportsTextResult>
  setNavigationText(
    params: SetNavigationTextParams,
  ): Promise<SetNavigationTextResult>
  setScopeText(params: SetScopeTextParams): Promise<SetScopeTextResult>
  setRuleSelector(params: SetRuleSelectorParams): Promise<SetRuleSelectorResult>
  setStyleSheetText(
    params: SetStyleSheetTextParams,
  ): Promise<SetStyleSheetTextResult>
  setStyleTexts(params: SetStyleTextsParams): Promise<SetStyleTextsResult>
  startRuleUsageTracking(): Promise<void>
  stopRuleUsageTracking(): Promise<StopRuleUsageTrackingResult>
  takeCoverageDelta(): Promise<TakeCoverageDeltaResult>
  setLocalFontsEnabled(params: SetLocalFontsEnabledParams): Promise<void>

  // ── Events ──

  on(
    event: 'fontsUpdated',
    handler: (params: FontsUpdatedEvent) => void,
  ): () => void
  on(event: 'mediaQueryResultChanged', handler: () => void): () => void
  on(
    event: 'styleSheetAdded',
    handler: (params: StyleSheetAddedEvent) => void,
  ): () => void
  on(
    event: 'styleSheetChanged',
    handler: (params: StyleSheetChangedEvent) => void,
  ): () => void
  on(
    event: 'styleSheetRemoved',
    handler: (params: StyleSheetRemovedEvent) => void,
  ): () => void
  on(
    event: 'computedStyleUpdated',
    handler: (params: ComputedStyleUpdatedEvent) => void,
  ): () => void
}
