// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  CallArgument,
  ExceptionDetails,
  ExecutionContextId,
  RemoteObject,
  RemoteObjectId,
  ScriptId,
  StackTrace,
  StackTraceId,
  TimeDelta,
  UniqueDebuggerId,
} from './runtime'

// ══ Types ══

export type BreakpointId = string

export type CallFrameId = string

export interface Location {
  scriptId: ScriptId
  lineNumber: number
  columnNumber?: number
}

export interface ScriptPosition {
  lineNumber: number
  columnNumber: number
}

export interface LocationRange {
  scriptId: ScriptId
  start: ScriptPosition
  end: ScriptPosition
}

export interface CallFrame {
  callFrameId: CallFrameId
  functionName: string
  functionLocation?: Location
  location: Location
  url: string
  scopeChain: Scope[]
  this: RemoteObject
  returnValue?: RemoteObject
  canBeRestarted?: boolean
}

export interface Scope {
  type:
    | 'global'
    | 'local'
    | 'with'
    | 'closure'
    | 'catch'
    | 'block'
    | 'script'
    | 'eval'
    | 'module'
    | 'wasm-expression-stack'
  object: RemoteObject
  name?: string
  startLocation?: Location
  endLocation?: Location
  empty?: boolean
}

export interface SearchMatch {
  lineNumber: number
  lineContent: string
}

export interface BreakLocation {
  scriptId: ScriptId
  lineNumber: number
  columnNumber?: number
  type?: 'debuggerStatement' | 'call' | 'return'
}

export interface WasmDisassemblyChunk {
  lines: string[]
  bytecodeOffsets: number[]
}

export type ScriptLanguage = 'JavaScript' | 'WebAssembly'

export interface DebugSymbols {
  type: 'SourceMap' | 'EmbeddedDWARF' | 'ExternalDWARF'
  externalURL?: string
}

export interface ResolvedBreakpoint {
  breakpointId: BreakpointId
  location: Location
}

// ══ Commands ══

export interface ContinueToLocationParams {
  location: Location
  targetCallFrames?: 'any' | 'current'
}

export interface EnableParams {
  maxScriptsCacheSize?: number
}

export interface EnableResult {
  debuggerId: UniqueDebuggerId
}

export interface EvaluateOnCallFrameParams {
  callFrameId: CallFrameId
  expression: string
  objectGroup?: string
  includeCommandLineAPI?: boolean
  silent?: boolean
  returnByValue?: boolean
  generatePreview?: boolean
  throwOnSideEffect?: boolean
  timeout?: TimeDelta
  scopeNumber?: number
}

export interface EvaluateOnCallFrameResult {
  result: RemoteObject
  exceptionDetails?: ExceptionDetails
}

export interface GetPossibleBreakpointsParams {
  start: Location
  end?: Location
  restrictToFunction?: boolean
}

export interface GetPossibleBreakpointsResult {
  locations: BreakLocation[]
}

export interface GetScriptSourceParams {
  scriptId: ScriptId
}

export interface GetScriptSourceResult {
  scriptSource: string
  bytecode?: string
}

export interface DisassembleWasmModuleParams {
  scriptId: ScriptId
}

export interface DisassembleWasmModuleResult {
  streamId?: string
  totalNumberOfLines: number
  functionBodyOffsets: number[]
  chunk: WasmDisassemblyChunk
}

export interface NextWasmDisassemblyChunkParams {
  streamId: string
}

export interface NextWasmDisassemblyChunkResult {
  chunk: WasmDisassemblyChunk
}

export interface GetWasmBytecodeParams {
  scriptId: ScriptId
}

export interface GetWasmBytecodeResult {
  bytecode: string
}

export interface GetStackTraceParams {
  stackTraceId: StackTraceId
}

export interface GetStackTraceResult {
  stackTrace: StackTrace
}

export interface PauseOnAsyncCallParams {
  parentStackTraceId: StackTraceId
}

export interface RemoveBreakpointParams {
  breakpointId: BreakpointId
}

export interface RestartFrameParams {
  callFrameId: CallFrameId
  mode?: 'StepInto'
}

export interface RestartFrameResult {
  callFrames: CallFrame[]
  asyncStackTrace?: StackTrace
  asyncStackTraceId?: StackTraceId
}

export interface ResumeParams {
  terminateOnResume?: boolean
}

export interface SearchInContentParams {
  scriptId: ScriptId
  query: string
  caseSensitive?: boolean
  isRegex?: boolean
}

export interface SearchInContentResult {
  result: SearchMatch[]
}

export interface SetAsyncCallStackDepthParams {
  maxDepth: number
}

export interface SetBlackboxExecutionContextsParams {
  uniqueIds: string[]
}

export interface SetBlackboxPatternsParams {
  patterns: string[]
  skipAnonymous?: boolean
}

export interface SetBlackboxedRangesParams {
  scriptId: ScriptId
  positions: ScriptPosition[]
}

export interface SetBreakpointParams {
  location: Location
  condition?: string
}

export interface SetBreakpointResult {
  breakpointId: BreakpointId
  actualLocation: Location
}

export interface SetInstrumentationBreakpointParams {
  instrumentation:
    | 'beforeScriptExecution'
    | 'beforeScriptWithSourceMapExecution'
}

export interface SetInstrumentationBreakpointResult {
  breakpointId: BreakpointId
}

export interface SetBreakpointByUrlParams {
  lineNumber: number
  url?: string
  urlRegex?: string
  scriptHash?: string
  columnNumber?: number
  condition?: string
}

export interface SetBreakpointByUrlResult {
  breakpointId: BreakpointId
  locations: Location[]
}

export interface SetBreakpointOnFunctionCallParams {
  objectId: RemoteObjectId
  condition?: string
}

export interface SetBreakpointOnFunctionCallResult {
  breakpointId: BreakpointId
}

export interface SetBreakpointsActiveParams {
  active: boolean
}

export interface SetPauseOnExceptionsParams {
  state: 'none' | 'caught' | 'uncaught' | 'all'
}

export interface SetReturnValueParams {
  newValue: CallArgument
}

export interface SetScriptSourceParams {
  scriptId: ScriptId
  scriptSource: string
  dryRun?: boolean
  allowTopFrameEditing?: boolean
}

export interface SetScriptSourceResult {
  callFrames?: CallFrame[]
  stackChanged?: boolean
  asyncStackTrace?: StackTrace
  asyncStackTraceId?: StackTraceId
  status:
    | 'Ok'
    | 'CompileError'
    | 'BlockedByActiveGenerator'
    | 'BlockedByActiveFunction'
    | 'BlockedByTopLevelEsModuleChange'
  exceptionDetails?: ExceptionDetails
}

export interface SetSkipAllPausesParams {
  skip: boolean
}

export interface SetVariableValueParams {
  scopeNumber: number
  variableName: string
  newValue: CallArgument
  callFrameId: CallFrameId
}

export interface StepIntoParams {
  breakOnAsyncCall?: boolean
  skipList?: LocationRange[]
}

export interface StepOverParams {
  skipList?: LocationRange[]
}

// ══ Events ══

export interface BreakpointResolvedEvent {
  breakpointId: BreakpointId
  location: Location
}

export interface PausedEvent {
  callFrames: CallFrame[]
  reason:
    | 'ambiguous'
    | 'assert'
    | 'CSPViolation'
    | 'debugCommand'
    | 'DOM'
    | 'EventListener'
    | 'exception'
    | 'instrumentation'
    | 'OOM'
    | 'other'
    | 'promiseRejection'
    | 'XHR'
    | 'step'
  data?: Record<string, unknown>
  hitBreakpoints?: string[]
  asyncStackTrace?: StackTrace
  asyncStackTraceId?: StackTraceId
  asyncCallStackTraceId?: StackTraceId
}

export interface ScriptFailedToParseEvent {
  scriptId: ScriptId
  url: string
  startLine: number
  startColumn: number
  endLine: number
  endColumn: number
  executionContextId: ExecutionContextId
  hash: string
  buildId: string
  executionContextAuxData?: Record<string, unknown>
  sourceMapURL?: string
  hasSourceURL?: boolean
  isModule?: boolean
  length?: number
  stackTrace?: StackTrace
  codeOffset?: number
  scriptLanguage?: ScriptLanguage
  embedderName?: string
}

export interface ScriptParsedEvent {
  scriptId: ScriptId
  url: string
  startLine: number
  startColumn: number
  endLine: number
  endColumn: number
  executionContextId: ExecutionContextId
  hash: string
  buildId: string
  executionContextAuxData?: Record<string, unknown>
  isLiveEdit?: boolean
  sourceMapURL?: string
  hasSourceURL?: boolean
  isModule?: boolean
  length?: number
  stackTrace?: StackTrace
  codeOffset?: number
  scriptLanguage?: ScriptLanguage
  debugSymbols?: DebugSymbols[]
  embedderName?: string
  resolvedBreakpoints?: ResolvedBreakpoint[]
}
