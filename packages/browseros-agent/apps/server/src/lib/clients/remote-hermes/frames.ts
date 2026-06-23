/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export const PING_FRAME = '{"type":"ping"}'

export type RpcServer = 'browseros' | 'browserclaw' | 'nudge'

export interface RpcRequestFrame {
  type: 'rpc.request'
  v: 1
  id: string
  server: RpcServer
  method: string
  params: unknown
}

export interface RpcResponseFrame {
  type: 'rpc.response'
  v: 1
  id: string
  result?: unknown
  error?: { code: string; message: string }
}

export interface PingFrame {
  type: 'ping'
}

export interface PongFrame {
  type: 'pong'
}

export type Frame = RpcRequestFrame | RpcResponseFrame | PingFrame | PongFrame

export function parseFrame(raw: string): Frame | null {
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return null
  }
  if (!isObject(parsed) || typeof parsed.type !== 'string') return null
  switch (parsed.type) {
    case 'ping':
      return { type: 'ping' }
    case 'pong':
      return { type: 'pong' }
    case 'rpc.request':
      if (
        parsed.v !== 1 ||
        typeof parsed.id !== 'string' ||
        typeof parsed.server !== 'string' ||
        typeof parsed.method !== 'string'
      ) {
        return null
      }
      return {
        type: 'rpc.request',
        v: 1,
        id: parsed.id,
        server: parsed.server as RpcServer,
        method: parsed.method,
        params: 'params' in parsed ? parsed.params : {},
      }
    case 'rpc.response':
      if (parsed.v !== 1 || typeof parsed.id !== 'string') return null
      return {
        type: 'rpc.response',
        v: 1,
        id: parsed.id,
        result: 'result' in parsed ? parsed.result : undefined,
        error: isObject(parsed.error)
          ? {
              code: String(
                (parsed.error as Record<string, unknown>).code ?? '',
              ),
              message: String(
                (parsed.error as Record<string, unknown>).message ?? '',
              ),
            }
          : undefined,
      }
    default:
      return null
  }
}

export function encodeFrame(frame: Frame): string {
  return JSON.stringify(frame)
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null
}
