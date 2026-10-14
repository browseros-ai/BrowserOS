// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

// ══ Types ══

export type ScriptId = string

export interface SerializationOptions {
  serialization: 'deep' | 'json' | 'idOnly'
  maxDepth?: number
  additionalParameters?: Record<string, unknown>
}

export interface DeepSerializedValue {
  type:
    | 'undefined'
    | 'null'
    | 'string'
    | 'number'
    | 'boolean'
    | 'bigint'
    | 'regexp'
    | 'date'
    | 'symbol'
    | 'array'
    | 'object'
    | 'function'
    | 'map'
    | 'set'
    | 'weakmap'
    | 'weakset'
    | 'error'
    | 'proxy'
    | 'promise'
    | 'typedarray'
    | 'arraybuffer'
    | 'node'
    | 'window'
    | 'generator'
  value?: unknown
  objectId?: string
  weakLocalObjectReference?: number
}

export type RemoteObjectId = string

export type UnserializableValue = string

export interface RemoteObject {
  type:
    | 'object'
    | 'function'
    | 'undefined'
    | 'string'
    | 'number'
    | 'boolean'
    | 'symbol'
    | 'bigint'
  subtype?:
    | 'array'
    | 'null'
    | 'node'
    | 'regexp'
    | 'date'
    | 'map'
    | 'set'
    | 'weakmap'
    | 'weakset'
    | 'iterator'
    | 'generator'
    | 'error'
    | 'proxy'
    | 'promise'
    | 'typedarray'
    | 'arraybuffer'
    | 'dataview'
    | 'webassemblymemory'
    | 'wasmvalue'
    | 'deferredmodule'
    | 'trustedtype'
  className?: string
  value?: unknown
  unserializableValue?: UnserializableValue
  description?: string
  deepSerializedValue?: DeepSerializedValue
  objectId?: RemoteObjectId
  preview?: ObjectPreview
  customPreview?: CustomPreview
}

export interface CustomPreview {
  header: string
  bodyGetterId?: RemoteObjectId
}

export interface ObjectPreview {
  type:
    | 'object'
    | 'function'
    | 'undefined'
    | 'string'
    | 'number'
    | 'boolean'
    | 'symbol'
    | 'bigint'
  subtype?:
    | 'array'
    | 'null'
    | 'node'
    | 'regexp'
    | 'date'
    | 'map'
    | 'set'
    | 'weakmap'
    | 'weakset'
    | 'iterator'
    | 'generator'
    | 'error'
    | 'proxy'
    | 'promise'
    | 'typedarray'
    | 'arraybuffer'
    | 'dataview'
    | 'webassemblymemory'
    | 'wasmvalue'
    | 'deferredmodule'
    | 'trustedtype'
  description?: string
  overflow: boolean
  properties: PropertyPreview[]
  entries?: EntryPreview[]
}

export interface PropertyPreview {
  name: string
  type:
    | 'object'
    | 'function'
    | 'undefined'
    | 'string'
    | 'number'
    | 'boolean'
    | 'symbol'
    | 'accessor'
    | 'bigint'
  value?: string
  valuePreview?: ObjectPreview
  subtype?:
    | 'array'
    | 'null'
    | 'node'
    | 'regexp'
    | 'date'
    | 'map'
    | 'set'
    | 'weakmap'
    | 'weakset'
    | 'iterator'
    | 'generator'
    | 'error'
    | 'proxy'
    | 'promise'
    | 'typedarray'
    | 'arraybuffer'
    | 'dataview'
    | 'webassemblymemory'
    | 'wasmvalue'
    | 'deferredmodule'
    | 'trustedtype'
}

export interface EntryPreview {
  key?: ObjectPreview
  value: ObjectPreview
}

export interface PropertyDescriptor {
  name: string
  value?: RemoteObject
  writable?: boolean
  get?: RemoteObject
  set?: RemoteObject
  configurable: boolean
  enumerable: boolean
  wasThrown?: boolean
  isOwn?: boolean
  symbol?: RemoteObject
}

export interface InternalPropertyDescriptor {
  name: string
  value?: RemoteObject
}

export interface PrivatePropertyDescriptor {
  name: string
  value?: RemoteObject
  get?: RemoteObject
  set?: RemoteObject
}

export interface CallArgument {
  value?: unknown
  unserializableValue?: UnserializableValue
  objectId?: RemoteObjectId
}

export type ExecutionContextId = number

export interface ExecutionContextDescription {
  id: ExecutionContextId
  origin: string
  name: string
  uniqueId: string
  auxData?: Record<string, unknown>
}

export interface ExceptionDetails {
  exceptionId: number
  text: string
  lineNumber: number
  columnNumber: number
  scriptId?: ScriptId
  url?: string
  stackTrace?: StackTrace
  exception?: RemoteObject
  executionContextId?: ExecutionContextId
  exceptionMetaData?: Record<string, unknown>
}

export type Timestamp = number

export type TimeDelta = number

export interface CallFrame {
  functionName: string
  scriptId: ScriptId
  url: string
  lineNumber: number
  columnNumber: number
}

export interface StackTrace {
  description?: string
  callFrames: CallFrame[]
  parent?: StackTrace
  parentId?: StackTraceId
}

export type UniqueDebuggerId = string

export interface StackTraceId {
  id: string
  debuggerId?: UniqueDebuggerId
}

// ══ Commands ══

export interface AwaitPromiseParams {
  promiseObjectId: RemoteObjectId
  returnByValue?: boolean
  generatePreview?: boolean
}

export interface AwaitPromiseResult {
  result: RemoteObject
  exceptionDetails?: ExceptionDetails
}

export interface CallFunctionOnParams {
  functionDeclaration: string
  objectId?: RemoteObjectId
  arguments?: CallArgument[]
  silent?: boolean
  returnByValue?: boolean
  generatePreview?: boolean
  userGesture?: boolean
  awaitPromise?: boolean
  executionContextId?: ExecutionContextId
  objectGroup?: string
  throwOnSideEffect?: boolean
  uniqueContextId?: string
  serializationOptions?: SerializationOptions
}

export interface CallFunctionOnResult {
  result: RemoteObject
  exceptionDetails?: ExceptionDetails
}

export interface CompileScriptParams {
  expression: string
  sourceURL: string
  persistScript: boolean
  executionContextId?: ExecutionContextId
}

export interface CompileScriptResult {
  scriptId?: ScriptId
  exceptionDetails?: ExceptionDetails
}

export interface EvaluateParams {
  expression: string
  objectGroup?: string
  includeCommandLineAPI?: boolean
  silent?: boolean
  contextId?: ExecutionContextId
  returnByValue?: boolean
  generatePreview?: boolean
  userGesture?: boolean
  awaitPromise?: boolean
  throwOnSideEffect?: boolean
  timeout?: TimeDelta
  disableBreaks?: boolean
  replMode?: boolean
  allowUnsafeEvalBlockedByCSP?: boolean
  uniqueContextId?: string
  serializationOptions?: SerializationOptions
}

export interface EvaluateResult {
  result: RemoteObject
  exceptionDetails?: ExceptionDetails
}

export interface GetIsolateIdResult {
  id: string
}

export interface GetHeapUsageResult {
  usedSize: number
  totalSize: number
  embedderHeapUsedSize: number
  backingStorageSize: number
}

export interface GetPropertiesParams {
  objectId: RemoteObjectId
  ownProperties?: boolean
  accessorPropertiesOnly?: boolean
  generatePreview?: boolean
  nonIndexedPropertiesOnly?: boolean
}

export interface GetPropertiesResult {
  result: PropertyDescriptor[]
  internalProperties?: InternalPropertyDescriptor[]
  privateProperties?: PrivatePropertyDescriptor[]
  exceptionDetails?: ExceptionDetails
}

export interface GlobalLexicalScopeNamesParams {
  executionContextId?: ExecutionContextId
}

export interface GlobalLexicalScopeNamesResult {
  names: string[]
}

export interface QueryObjectsParams {
  prototypeObjectId: RemoteObjectId
  objectGroup?: string
}

export interface QueryObjectsResult {
  objects: RemoteObject
}

export interface ReleaseObjectParams {
  objectId: RemoteObjectId
}

export interface ReleaseObjectGroupParams {
  objectGroup: string
}

export interface RunScriptParams {
  scriptId: ScriptId
  executionContextId?: ExecutionContextId
  objectGroup?: string
  silent?: boolean
  includeCommandLineAPI?: boolean
  returnByValue?: boolean
  generatePreview?: boolean
  awaitPromise?: boolean
}

export interface RunScriptResult {
  result: RemoteObject
  exceptionDetails?: ExceptionDetails
}

export interface SetAsyncCallStackDepthParams {
  maxDepth: number
}

export interface SetCustomObjectFormatterEnabledParams {
  enabled: boolean
}

export interface SetMaxCallStackSizeToCaptureParams {
  size: number
}

export interface AddBindingParams {
  name: string
  executionContextId?: ExecutionContextId
  executionContextName?: string
}

export interface RemoveBindingParams {
  name: string
}

export interface GetExceptionDetailsParams {
  errorObjectId: RemoteObjectId
}

export interface GetExceptionDetailsResult {
  exceptionDetails?: ExceptionDetails
}

// ══ Events ══

export interface BindingCalledEvent {
  name: string
  payload: string
  executionContextId: ExecutionContextId
}

export interface ConsoleAPICalledEvent {
  type:
    | 'log'
    | 'debug'
    | 'info'
    | 'error'
    | 'warning'
    | 'dir'
    | 'dirxml'
    | 'table'
    | 'trace'
    | 'clear'
    | 'startGroup'
    | 'startGroupCollapsed'
    | 'endGroup'
    | 'assert'
    | 'profile'
    | 'profileEnd'
    | 'count'
    | 'timeEnd'
  args: RemoteObject[]
  executionContextId: ExecutionContextId
  timestamp: Timestamp
  stackTrace?: StackTrace
  context?: string
}

export interface ExceptionRevokedEvent {
  reason: string
  exceptionId: number
}

export interface ExceptionThrownEvent {
  timestamp: Timestamp
  exceptionDetails: ExceptionDetails
}

export interface ExecutionContextCreatedEvent {
  context: ExecutionContextDescription
}

export interface ExecutionContextDestroyedEvent {
  executionContextId: ExecutionContextId
  executionContextUniqueId: string
}

export interface InspectRequestedEvent {
  object: RemoteObject
  hints: Record<string, unknown>
  executionContextId?: ExecutionContextId
}
