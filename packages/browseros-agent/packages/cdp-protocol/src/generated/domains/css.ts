// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  BackendNodeId,
  LogicalAxes,
  NodeId,
  PhysicalAxes,
  PseudoType,
  StyleSheetId,
} from './dom'
import type { FrameId } from './page'

// ══ Types ══

export type StyleSheetOrigin =
  | 'injected'
  | 'user-agent'
  | 'inspector'
  | 'regular'

export interface PseudoElementMatches {
  pseudoType: PseudoType
  pseudoIdentifier?: string
  matches: RuleMatch[]
}

export interface CSSAnimationStyle {
  name?: string
  style: CSSStyle
}

export interface InheritedStyleEntry {
  inlineStyle?: CSSStyle
  matchedCSSRules: RuleMatch[]
}

export interface InheritedAnimatedStyleEntry {
  animationStyles?: CSSAnimationStyle[]
  transitionsStyle?: CSSStyle
}

export interface InheritedPseudoElementMatches {
  pseudoElements: PseudoElementMatches[]
}

export interface RuleMatch {
  rule: CSSRule
  matchingSelectors: number[]
}

export interface Value {
  text: string
  range?: SourceRange
  specificity?: Specificity
}

export interface Specificity {
  a: number
  b: number
  c: number
}

export interface SelectorList {
  selectors: Value[]
  text: string
}

export interface CSSStyleSheetHeader {
  styleSheetId: StyleSheetId
  frameId: FrameId
  sourceURL: string
  sourceMapURL?: string
  origin: StyleSheetOrigin
  title: string
  ownerNode?: BackendNodeId
  disabled: boolean
  hasSourceURL?: boolean
  isInline: boolean
  isMutable: boolean
  isConstructed: boolean
  startLine: number
  startColumn: number
  length: number
  endLine: number
  endColumn: number
  loadingFailed?: boolean
}

export interface CSSRule {
  styleSheetId?: StyleSheetId
  selectorList: SelectorList
  nestingSelectors?: string[]
  origin: StyleSheetOrigin
  style: CSSStyle
  originTreeScopeNodeId?: BackendNodeId
  media?: CSSMedia[]
  containerQueries?: CSSContainerQuery[]
  supports?: CSSSupports[]
  layers?: CSSLayer[]
  scopes?: CSSScope[]
  ruleTypes?: CSSRuleType[]
  startingStyles?: CSSStartingStyle[]
  navigations?: CSSNavigation[]
}

export type CSSRuleType =
  | 'MediaRule'
  | 'SupportsRule'
  | 'ContainerRule'
  | 'LayerRule'
  | 'ScopeRule'
  | 'StyleRule'
  | 'StartingStyleRule'
  | 'NavigationRule'

export interface RuleUsage {
  styleSheetId: StyleSheetId
  startOffset: number
  endOffset: number
  used: boolean
}

export interface SourceRange {
  startLine: number
  startColumn: number
  endLine: number
  endColumn: number
}

export interface ShorthandEntry {
  name: string
  value: string
  important?: boolean
}

export interface CSSComputedStyleProperty {
  name: string
  value: string
}

export interface ComputedStyleExtraFields {
  isAppearanceBase: boolean
}

export interface CSSStyle {
  styleSheetId?: StyleSheetId
  cssProperties: CSSProperty[]
  shorthandEntries: ShorthandEntry[]
  cssText?: string
  range?: SourceRange
}

export interface CSSProperty {
  name: string
  value: string
  important?: boolean
  implicit?: boolean
  text?: string
  parsedOk?: boolean
  disabled?: boolean
  range?: SourceRange
  longhandProperties?: CSSProperty[]
}

export interface CSSMedia {
  text: string
  source: 'mediaRule' | 'importRule' | 'linkedSheet' | 'inlineSheet'
  sourceURL?: string
  range?: SourceRange
  styleSheetId?: StyleSheetId
  mediaList?: MediaQuery[]
}

export interface MediaQuery {
  expressions: MediaQueryExpression[]
  active: boolean
}

export interface MediaQueryExpression {
  value: number
  unit: string
  feature: string
  valueRange?: SourceRange
  computedLength?: number
}

export interface CSSContainerQuery {
  text: string
  range?: SourceRange
  styleSheetId?: StyleSheetId
  name?: string
  physicalAxes?: PhysicalAxes
  logicalAxes?: LogicalAxes
  queriesScrollState?: boolean
  queriesAnchored?: boolean
}

export interface CSSSupports {
  text: string
  active: boolean
  range?: SourceRange
  styleSheetId?: StyleSheetId
}

export interface CSSNavigation {
  text: string
  active?: boolean
  range?: SourceRange
  styleSheetId?: StyleSheetId
}

export interface CSSScope {
  text: string
  range?: SourceRange
  styleSheetId?: StyleSheetId
}

export interface CSSLayer {
  text: string
  range?: SourceRange
  styleSheetId?: StyleSheetId
}

export interface CSSStartingStyle {
  range?: SourceRange
  styleSheetId?: StyleSheetId
}

export interface CSSLayerData {
  name: string
  subLayers?: CSSLayerData[]
  order: number
}

export interface PlatformFontUsage {
  familyName: string
  postScriptName: string
  isCustomFont: boolean
  glyphCount: number
}

export interface FontVariationAxis {
  tag: string
  name: string
  minValue: number
  maxValue: number
  defaultValue: number
}

export interface FontFace {
  fontFamily: string
  fontStyle: string
  fontVariant: string
  fontWeight: string
  fontStretch: string
  fontDisplay: string
  unicodeRange: string
  src: string
  platformFontFamily: string
  fontVariationAxes?: FontVariationAxis[]
}

export interface CSSTryRule {
  styleSheetId?: StyleSheetId
  origin: StyleSheetOrigin
  style: CSSStyle
}

export interface CSSPositionTryRule {
  name: Value
  styleSheetId?: StyleSheetId
  origin: StyleSheetOrigin
  style: CSSStyle
  active: boolean
}

export interface CSSKeyframesRule {
  animationName: Value
  keyframes: CSSKeyframeRule[]
}

export interface CSSPropertyRegistration {
  propertyName: string
  initialValue?: Value
  inherits: boolean
  syntax: string
}

export interface CSSAtRule {
  type: 'font-face' | 'font-feature-values' | 'font-palette-values'
  subsection?:
    | 'swash'
    | 'annotation'
    | 'ornaments'
    | 'stylistic'
    | 'styleset'
    | 'character-variant'
  name?: Value
  styleSheetId?: StyleSheetId
  origin: StyleSheetOrigin
  style: CSSStyle
}

export interface CSSPropertyRule {
  styleSheetId?: StyleSheetId
  origin: StyleSheetOrigin
  propertyName: Value
  style: CSSStyle
}

export interface CSSFunctionParameter {
  name: string
  type: string
}

export interface CSSFunctionConditionNode {
  media?: CSSMedia
  containerQueries?: CSSContainerQuery
  supports?: CSSSupports
  navigation?: CSSNavigation
  children: CSSFunctionNode[]
  conditionText: string
}

export interface CSSFunctionNode {
  condition?: CSSFunctionConditionNode
  style?: CSSStyle
}

export interface CSSFunctionRule {
  name: Value
  styleSheetId?: StyleSheetId
  origin: StyleSheetOrigin
  parameters: CSSFunctionParameter[]
  children: CSSFunctionNode[]
  originTreeScopeNodeId?: BackendNodeId
}

export interface CSSKeyframeRule {
  styleSheetId?: StyleSheetId
  origin: StyleSheetOrigin
  keyText: Value
  style: CSSStyle
}

export interface StyleDeclarationEdit {
  styleSheetId: StyleSheetId
  range: SourceRange
  text: string
}

// ══ Commands ══

export interface AddRuleParams {
  styleSheetId: StyleSheetId
  ruleText: string
  location: SourceRange
  nodeForPropertySyntaxValidation?: NodeId
}

export interface AddRuleResult {
  rule: CSSRule
}

export interface CollectClassNamesParams {
  styleSheetId: StyleSheetId
}

export interface CollectClassNamesResult {
  classNames: string[]
}

export interface CreateStyleSheetParams {
  frameId: FrameId
  force?: boolean
}

export interface CreateStyleSheetResult {
  styleSheetId: StyleSheetId
}

export interface ForcePseudoStateParams {
  nodeId: NodeId
  forcedPseudoClasses: string[]
}

export interface ForceStartingStyleParams {
  nodeId: NodeId
  forced: boolean
}

export interface GetBackgroundColorsParams {
  nodeId: NodeId
}

export interface GetBackgroundColorsResult {
  backgroundColors?: string[]
  computedFontSize?: string
  computedFontWeight?: string
}

export interface GetComputedStyleForNodeParams {
  nodeId: NodeId
}

export interface GetComputedStyleForNodeResult {
  computedStyle: CSSComputedStyleProperty[]
  extraFields: ComputedStyleExtraFields
}

export interface ResolveValuesParams {
  values: string[]
  nodeId: NodeId
  propertyName?: string
  pseudoType?: PseudoType
  pseudoIdentifier?: string
}

export interface ResolveValuesResult {
  results: string[]
}

export interface GetLonghandPropertiesParams {
  shorthandName: string
  value: string
}

export interface GetLonghandPropertiesResult {
  longhandProperties: CSSProperty[]
}

export interface GetInlineStylesForNodeParams {
  nodeId: NodeId
}

export interface GetInlineStylesForNodeResult {
  inlineStyle?: CSSStyle
  attributesStyle?: CSSStyle
}

export interface GetAnimatedStylesForNodeParams {
  nodeId: NodeId
}

export interface GetAnimatedStylesForNodeResult {
  animationStyles?: CSSAnimationStyle[]
  transitionsStyle?: CSSStyle
  inherited?: InheritedAnimatedStyleEntry[]
}

export interface GetMatchedStylesForNodeParams {
  nodeId: NodeId
}

export interface GetMatchedStylesForNodeResult {
  inlineStyle?: CSSStyle
  attributesStyle?: CSSStyle
  matchedCSSRules?: RuleMatch[]
  pseudoElements?: PseudoElementMatches[]
  inherited?: InheritedStyleEntry[]
  inheritedPseudoElements?: InheritedPseudoElementMatches[]
  cssKeyframesRules?: CSSKeyframesRule[]
  cssPositionTryRules?: CSSPositionTryRule[]
  activePositionFallbackIndex?: number
  cssPropertyRules?: CSSPropertyRule[]
  cssPropertyRegistrations?: CSSPropertyRegistration[]
  cssAtRules?: CSSAtRule[]
  parentLayoutNodeId?: NodeId
  cssFunctionRules?: CSSFunctionRule[]
}

export interface GetEnvironmentVariablesResult {
  environmentVariables: Record<string, unknown>
}

export interface GetMediaQueriesResult {
  medias: CSSMedia[]
}

export interface GetPlatformFontsForNodeParams {
  nodeId: NodeId
}

export interface GetPlatformFontsForNodeResult {
  fonts: PlatformFontUsage[]
}

export interface GetStyleSheetTextParams {
  styleSheetId: StyleSheetId
}

export interface GetStyleSheetTextResult {
  text: string
}

export interface GetLayersForNodeParams {
  nodeId: NodeId
}

export interface GetLayersForNodeResult {
  rootLayer: CSSLayerData
}

export interface GetLocationForSelectorParams {
  styleSheetId: StyleSheetId
  selectorText: string
}

export interface GetLocationForSelectorResult {
  ranges: SourceRange[]
}

export interface TrackComputedStyleUpdatesForNodeParams {
  nodeId?: NodeId
}

export interface TrackComputedStyleUpdatesParams {
  propertiesToTrack: CSSComputedStyleProperty[]
}

export interface TakeComputedStyleUpdatesResult {
  nodeIds: NodeId[]
}

export interface SetEffectivePropertyValueForNodeParams {
  nodeId: NodeId
  propertyName: string
  value: string
}

export interface SetPropertyRulePropertyNameParams {
  styleSheetId: StyleSheetId
  range: SourceRange
  propertyName: string
}

export interface SetPropertyRulePropertyNameResult {
  propertyName: Value
}

export interface SetKeyframeKeyParams {
  styleSheetId: StyleSheetId
  range: SourceRange
  keyText: string
}

export interface SetKeyframeKeyResult {
  keyText: Value
}

export interface SetMediaTextParams {
  styleSheetId: StyleSheetId
  range: SourceRange
  text: string
}

export interface SetMediaTextResult {
  media: CSSMedia
}

export interface SetContainerQueryTextParams {
  styleSheetId: StyleSheetId
  range: SourceRange
  text: string
}

export interface SetContainerQueryTextResult {
  containerQuery: CSSContainerQuery
}

export interface SetSupportsTextParams {
  styleSheetId: StyleSheetId
  range: SourceRange
  text: string
}

export interface SetSupportsTextResult {
  supports: CSSSupports
}

export interface SetNavigationTextParams {
  styleSheetId: StyleSheetId
  range: SourceRange
  text: string
}

export interface SetNavigationTextResult {
  navigation: CSSNavigation
}

export interface SetScopeTextParams {
  styleSheetId: StyleSheetId
  range: SourceRange
  text: string
}

export interface SetScopeTextResult {
  scope: CSSScope
}

export interface SetRuleSelectorParams {
  styleSheetId: StyleSheetId
  range: SourceRange
  selector: string
}

export interface SetRuleSelectorResult {
  selectorList: SelectorList
}

export interface SetStyleSheetTextParams {
  styleSheetId: StyleSheetId
  text: string
}

export interface SetStyleSheetTextResult {
  sourceMapURL?: string
}

export interface SetStyleTextsParams {
  edits: StyleDeclarationEdit[]
  nodeForPropertySyntaxValidation?: NodeId
}

export interface SetStyleTextsResult {
  styles: CSSStyle[]
}

export interface StopRuleUsageTrackingResult {
  ruleUsage: RuleUsage[]
}

export interface TakeCoverageDeltaResult {
  coverage: RuleUsage[]
  timestamp: number
}

export interface SetLocalFontsEnabledParams {
  enabled: boolean
}

// ══ Events ══

export interface FontsUpdatedEvent {
  font?: FontFace
}

export interface StyleSheetAddedEvent {
  header: CSSStyleSheetHeader
}

export interface StyleSheetChangedEvent {
  styleSheetId: StyleSheetId
}

export interface StyleSheetRemovedEvent {
  styleSheetId: StyleSheetId
}

export interface ComputedStyleUpdatedEvent {
  nodeId: NodeId
}
