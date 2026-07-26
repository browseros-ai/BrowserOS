import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'

export interface CdpBackend extends ProtocolApi {
  connect(): Promise<void>
  disconnect(): Promise<void>
  isConnected(): boolean
  connectionEpoch(): number
  getTargets(): Promise<CdpTarget[]>
  session(sessionId: string): ProtocolApi
  rawSend(
    method: string,
    params?: Record<string, unknown>,
    sessionId?: string,
  ): Promise<unknown>
  rawSendJson(
    method: string,
    paramsJson: string,
    sessionId?: string,
  ): Promise<unknown>
  onSessionEvent(
    event: string,
    handler: (params: unknown, sessionId: string) => void,
  ): () => void
}

export interface CdpTarget {
  id: string
  type: string
  title: string
  url: string
  tabId?: number
  windowId?: number
}
