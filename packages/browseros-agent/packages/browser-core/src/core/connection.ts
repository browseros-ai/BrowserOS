import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'

export type SessionId = string
export type FrameId = string

/**
 * The CDP capability browser-core needs: the root ProtocolApi (custom BrowserOS domains
 * included) plus the ability to address a per-target session. The concrete websocket client
 * lives in the host app and is injected, so browser-core never imports it (no dependency cycle).
 */
export interface CdpConnection extends ProtocolApi {
  session(sessionId: SessionId): ProtocolApi
  rawSend(
    method: string,
    params?: Record<string, unknown>,
    sessionId?: SessionId,
  ): Promise<unknown>
  rawSendJson(
    method: string,
    paramsJson: string,
    sessionId?: SessionId,
  ): Promise<unknown>
  isConnected(): boolean
  connectionEpoch(): number
}

/** Internal/agent surfaces we never expose as pages. */
export const EXCLUDED_URL_PREFIXES: readonly string[] = [
  'chrome-extension://',
  'chrome-untrusted://',
  'chrome-search://',
  'devtools://',
]
