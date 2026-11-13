/**
 * The live screen's one way to Neo's host API. In the running viewer it is the shared host API
 * client (`server/hostApi.ts`: address and token from the viewer's configuration); tests hand
 * the live-screen routes a fake Neo instead.
 */

import { hostFetch, hostWebSocketTarget } from '../hostApi'

/**
 * The codes the host API client answers with, in Neo's error shape, when there is no Neo to ask:
 * none configured (`503 no_host_api`) or nothing answering (`502 host_api_unreachable`). The live
 * screen reads both as "no container".
 */
export const NO_HOST = new Set(['no_host_api', 'host_api_unreachable'])

export interface LiveHost {
  /**
   * A host API request. When Neo cannot be reached it answers with a `NO_HOST` code, or
   * rejects; the live screen takes either.
   */
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
