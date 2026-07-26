// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  BeginTransactionRequestedEvent,
  CancelRequestedEvent,
  ConnectRequestedEvent,
  ControlRequestedEvent,
  DisconnectRequestedEvent,
  EndTransactionRequestedEvent,
  EstablishContextRequestedEvent,
  GetAttribRequestedEvent,
  GetStatusChangeRequestedEvent,
  ListReadersRequestedEvent,
  ReleaseContextRequestedEvent,
  ReportBeginTransactionResultParams,
  ReportConnectResultParams,
  ReportDataResultParams,
  ReportErrorParams,
  ReportEstablishContextResultParams,
  ReportGetStatusChangeResultParams,
  ReportListReadersResultParams,
  ReportPlainResultParams,
  ReportReleaseContextResultParams,
  ReportStatusResultParams,
  SetAttribRequestedEvent,
  StatusRequestedEvent,
  TransmitRequestedEvent,
} from '../domains/smart-card-emulation'

export interface SmartCardEmulationApi {
  // ── Commands ──

  enable(): Promise<void>
  disable(): Promise<void>
  reportEstablishContextResult(
    params: ReportEstablishContextResultParams,
  ): Promise<void>
  reportReleaseContextResult(
    params: ReportReleaseContextResultParams,
  ): Promise<void>
  reportListReadersResult(params: ReportListReadersResultParams): Promise<void>
  reportGetStatusChangeResult(
    params: ReportGetStatusChangeResultParams,
  ): Promise<void>
  reportBeginTransactionResult(
    params: ReportBeginTransactionResultParams,
  ): Promise<void>
  reportPlainResult(params: ReportPlainResultParams): Promise<void>
  reportConnectResult(params: ReportConnectResultParams): Promise<void>
  reportDataResult(params: ReportDataResultParams): Promise<void>
  reportStatusResult(params: ReportStatusResultParams): Promise<void>
  reportError(params: ReportErrorParams): Promise<void>

  // ── Events ──

  on(
    event: 'establishContextRequested',
    handler: (params: EstablishContextRequestedEvent) => void,
  ): () => void
  on(
    event: 'releaseContextRequested',
    handler: (params: ReleaseContextRequestedEvent) => void,
  ): () => void
  on(
    event: 'listReadersRequested',
    handler: (params: ListReadersRequestedEvent) => void,
  ): () => void
  on(
    event: 'getStatusChangeRequested',
    handler: (params: GetStatusChangeRequestedEvent) => void,
  ): () => void
  on(
    event: 'cancelRequested',
    handler: (params: CancelRequestedEvent) => void,
  ): () => void
  on(
    event: 'connectRequested',
    handler: (params: ConnectRequestedEvent) => void,
  ): () => void
  on(
    event: 'disconnectRequested',
    handler: (params: DisconnectRequestedEvent) => void,
  ): () => void
  on(
    event: 'transmitRequested',
    handler: (params: TransmitRequestedEvent) => void,
  ): () => void
  on(
    event: 'controlRequested',
    handler: (params: ControlRequestedEvent) => void,
  ): () => void
  on(
    event: 'getAttribRequested',
    handler: (params: GetAttribRequestedEvent) => void,
  ): () => void
  on(
    event: 'setAttribRequested',
    handler: (params: SetAttribRequestedEvent) => void,
  ): () => void
  on(
    event: 'statusRequested',
    handler: (params: StatusRequestedEvent) => void,
  ): () => void
  on(
    event: 'beginTransactionRequested',
    handler: (params: BeginTransactionRequestedEvent) => void,
  ): () => void
  on(
    event: 'endTransactionRequested',
    handler: (params: EndTransactionRequestedEvent) => void,
  ): () => void
}
