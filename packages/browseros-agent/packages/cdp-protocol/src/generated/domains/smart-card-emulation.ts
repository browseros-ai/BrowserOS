// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

// ══ Types ══

export type ResultCode =
  | 'success'
  | 'removed-card'
  | 'reset-card'
  | 'unpowered-card'
  | 'unresponsive-card'
  | 'unsupported-card'
  | 'reader-unavailable'
  | 'sharing-violation'
  | 'not-transacted'
  | 'no-smartcard'
  | 'proto-mismatch'
  | 'system-cancelled'
  | 'not-ready'
  | 'cancelled'
  | 'insufficient-buffer'
  | 'invalid-handle'
  | 'invalid-parameter'
  | 'invalid-value'
  | 'no-memory'
  | 'timeout'
  | 'unknown-reader'
  | 'unsupported-feature'
  | 'no-readers-available'
  | 'service-stopped'
  | 'no-service'
  | 'comm-error'
  | 'internal-error'
  | 'server-too-busy'
  | 'unexpected'
  | 'shutdown'
  | 'unknown-card'
  | 'unknown'

export type ShareMode = 'shared' | 'exclusive' | 'direct'

export type Disposition =
  | 'leave-card'
  | 'reset-card'
  | 'unpower-card'
  | 'eject-card'

export type ConnectionState =
  | 'absent'
  | 'present'
  | 'swallowed'
  | 'powered'
  | 'negotiable'
  | 'specific'

export interface ReaderStateFlags {
  unaware?: boolean
  ignore?: boolean
  changed?: boolean
  unknown?: boolean
  unavailable?: boolean
  empty?: boolean
  present?: boolean
  exclusive?: boolean
  inuse?: boolean
  mute?: boolean
  unpowered?: boolean
}

export interface ProtocolSet {
  t0?: boolean
  t1?: boolean
  raw?: boolean
}

export type Protocol = 't0' | 't1' | 'raw'

export interface ReaderStateIn {
  reader: string
  currentState: ReaderStateFlags
  currentInsertionCount: number
}

export interface ReaderStateOut {
  reader: string
  eventState: ReaderStateFlags
  eventCount: number
  atr: string
}

// ══ Commands ══

export interface ReportEstablishContextResultParams {
  requestId: string
  contextId: number
}

export interface ReportReleaseContextResultParams {
  requestId: string
}

export interface ReportListReadersResultParams {
  requestId: string
  readers: string[]
}

export interface ReportGetStatusChangeResultParams {
  requestId: string
  readerStates: ReaderStateOut[]
}

export interface ReportBeginTransactionResultParams {
  requestId: string
  handle: number
}

export interface ReportPlainResultParams {
  requestId: string
}

export interface ReportConnectResultParams {
  requestId: string
  handle: number
  activeProtocol?: Protocol
}

export interface ReportDataResultParams {
  requestId: string
  data: string
}

export interface ReportStatusResultParams {
  requestId: string
  readerName: string
  state: ConnectionState
  atr: string
  protocol?: Protocol
}

export interface ReportErrorParams {
  requestId: string
  resultCode: ResultCode
}

// ══ Events ══

export interface EstablishContextRequestedEvent {
  requestId: string
}

export interface ReleaseContextRequestedEvent {
  requestId: string
  contextId: number
}

export interface ListReadersRequestedEvent {
  requestId: string
  contextId: number
}

export interface GetStatusChangeRequestedEvent {
  requestId: string
  contextId: number
  readerStates: ReaderStateIn[]
  timeout?: number
}

export interface CancelRequestedEvent {
  requestId: string
  contextId: number
}

export interface ConnectRequestedEvent {
  requestId: string
  contextId: number
  reader: string
  shareMode: ShareMode
  preferredProtocols: ProtocolSet
}

export interface DisconnectRequestedEvent {
  requestId: string
  handle: number
  disposition: Disposition
}

export interface TransmitRequestedEvent {
  requestId: string
  handle: number
  data: string
  protocol?: Protocol
}

export interface ControlRequestedEvent {
  requestId: string
  handle: number
  controlCode: number
  data: string
}

export interface GetAttribRequestedEvent {
  requestId: string
  handle: number
  attribId: number
}

export interface SetAttribRequestedEvent {
  requestId: string
  handle: number
  attribId: number
  data: string
}

export interface StatusRequestedEvent {
  requestId: string
  handle: number
}

export interface BeginTransactionRequestedEvent {
  requestId: string
  handle: number
}

export interface EndTransactionRequestedEvent {
  requestId: string
  handle: number
  disposition: Disposition
}
