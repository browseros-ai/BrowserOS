// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

// ══ Types ══

export type GraphObjectId = string

export type ContextType = 'realtime' | 'offline'

export type ContextState = 'suspended' | 'running' | 'closed' | 'interrupted'

export type NodeType = string

export type ChannelCountMode = 'clamped-max' | 'explicit' | 'max'

export type ChannelInterpretation = 'discrete' | 'speakers'

export type ParamType = string

export type AutomationRate = 'a-rate' | 'k-rate'

export interface ContextRealtimeData {
  currentTime: number
  renderCapacity: number
  callbackIntervalMean: number
  callbackIntervalVariance: number
}

export interface BaseAudioContext {
  contextId: GraphObjectId
  contextType: ContextType
  contextState: ContextState
  realtimeData?: ContextRealtimeData
  callbackBufferSize: number
  maxOutputChannelCount: number
  sampleRate: number
  renderQuantumSize: number
}

export interface AudioListener {
  listenerId: GraphObjectId
  contextId: GraphObjectId
}

export interface AudioNode {
  nodeId: GraphObjectId
  contextId: GraphObjectId
  nodeType: NodeType
  numberOfInputs: number
  numberOfOutputs: number
  channelCount: number
  channelCountMode: ChannelCountMode
  channelInterpretation: ChannelInterpretation
}

export interface AudioParam {
  paramId: GraphObjectId
  nodeId: GraphObjectId
  contextId: GraphObjectId
  paramType: ParamType
  rate: AutomationRate
  defaultValue: number
  minValue: number
  maxValue: number
}

// ══ Commands ══

export interface GetRealtimeDataParams {
  contextId: GraphObjectId
}

export interface GetRealtimeDataResult {
  realtimeData: ContextRealtimeData
}

// ══ Events ══

export interface ContextCreatedEvent {
  context: BaseAudioContext
}

export interface ContextWillBeDestroyedEvent {
  contextId: GraphObjectId
}

export interface ContextChangedEvent {
  context: BaseAudioContext
}

export interface AudioListenerCreatedEvent {
  listener: AudioListener
}

export interface AudioListenerWillBeDestroyedEvent {
  contextId: GraphObjectId
  listenerId: GraphObjectId
}

export interface AudioNodeCreatedEvent {
  node: AudioNode
}

export interface AudioNodeWillBeDestroyedEvent {
  contextId: GraphObjectId
  nodeId: GraphObjectId
}

export interface AudioParamCreatedEvent {
  param: AudioParam
}

export interface AudioParamWillBeDestroyedEvent {
  contextId: GraphObjectId
  nodeId: GraphObjectId
  paramId: GraphObjectId
}

export interface NodesConnectedEvent {
  contextId: GraphObjectId
  sourceId: GraphObjectId
  destinationId: GraphObjectId
  sourceOutputIndex?: number
  destinationInputIndex?: number
}

export interface NodesDisconnectedEvent {
  contextId: GraphObjectId
  sourceId: GraphObjectId
  destinationId: GraphObjectId
  sourceOutputIndex?: number
  destinationInputIndex?: number
}

export interface NodeParamConnectedEvent {
  contextId: GraphObjectId
  sourceId: GraphObjectId
  destinationId: GraphObjectId
  sourceOutputIndex?: number
}

export interface NodeParamDisconnectedEvent {
  contextId: GraphObjectId
  sourceId: GraphObjectId
  destinationId: GraphObjectId
  sourceOutputIndex?: number
}
