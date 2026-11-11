/**
 * Bun serves one WebSocket handler per server. The viewer's handler is this dispatcher: each
 * upgraded socket carries its own session object as `ws.data`, so a route decides what a socket
 * does when it upgrades (the display relay is one such session) and the server stays generic.
 */

import type { ServerWebSocket, WebSocketHandler } from 'bun'

export type SessionSocket = ServerWebSocket<SocketSession>

export interface SocketSession {
  open(ws: SessionSocket): void
  message(ws: SessionSocket, message: string | Buffer): void
  close(ws: SessionSocket, code: number, reason: string): void
  /** The socket can take more data after `send` reported backpressure. */
  drain?(ws: SessionSocket): void
}

export const socketSessions: WebSocketHandler<SocketSession> = {
  data: {} as SocketSession,
  open: (ws) => ws.data.open(ws),
  message: (ws, message) => ws.data.message(ws, message),
  close: (ws, code, reason) => ws.data.close(ws, code, reason),
  drain: (ws) => ws.data.drain?.(ws),
  // A VNC screen at rest can stay silent for minutes; Bun's pings keep the socket alive and a
  // page that stops answering them is still closed.
  sendPings: true,
}
