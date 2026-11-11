/**
 * The live screen's one way to Neo's host API. In the running viewer it is the shared host API
 * client (`server/hostApi.ts`: address and token from the viewer's configuration); tests hand
 * the live-screen routes a fake Neo instead.
 */

import { hostFetch, hostWebSocketTarget } from '../hostApi'

export interface LiveHost {
  /** A host API request; rejects when Neo cannot be reached. */
  fetch(path: string, init?: RequestInit): Promise<Response>
  /**
   * Where to open a host API WebSocket. `headers` carries `Authorization`: Neo refuses a token
   * in the query string, and the token never reaches the browser.
   */
  socket(path: string): { url: string; headers: Record<string, string> }
}

export const viewerHost: LiveHost = {
  fetch: hostFetch,
  socket: hostWebSocketTarget,
}
