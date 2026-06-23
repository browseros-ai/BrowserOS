/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */
import { logger } from '../../logger'
import type { RpcRequestFrame, RpcResponseFrame } from './frames'

const MODULE = 'remote-hermes'

export interface RpcRouterDeps {
  /** Returns the local MCP base URL for a given logical server name. */
  resolveBaseUrl(server: string): string | null
  /** Scope id attached as `X-BrowserOS-Scope-Id` for local MCP attribution. */
  scopeId: string
  /** Provider id forwarded as `X-BrowserOS-Agent-Id` for audit. */
  agentId: string
}

interface JsonRpcResponse {
  jsonrpc: '2.0'
  id: string | number
  result?: unknown
  error?: { code: number; message: string; data?: unknown }
}

export async function dispatchRpcRequest(
  frame: RpcRequestFrame,
  deps: RpcRouterDeps,
): Promise<RpcResponseFrame> {
  const base = deps.resolveBaseUrl(frame.server)
  if (!base) {
    return errorFrame(
      frame.id,
      'unknown_server',
      new Error(`no local route for server '${frame.server}'`),
    )
  }
  let res: Response
  try {
    res = await fetch(base, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        accept: 'application/json, text/event-stream',
        'X-BrowserOS-Scope-Id': deps.scopeId,
        'X-BrowserOS-Agent-Id': deps.agentId,
      },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: frame.id,
        method: frame.method,
        params: frame.params ?? {},
      }),
    })
  } catch (err) {
    logger.warn('Remote Hermes RPC local dispatch failed', {
      module: MODULE,
      server: frame.server,
      err: err instanceof Error ? err.message : String(err),
    })
    return errorFrame(frame.id, 'local_dispatch_failed', err)
  }
  let json: JsonRpcResponse
  try {
    json = (await res.json()) as JsonRpcResponse
  } catch {
    return errorFrame(
      frame.id,
      'bad_local_response',
      new Error(`non-JSON response from ${frame.server}`),
    )
  }
  if (json.error) {
    return {
      type: 'rpc.response',
      v: 1,
      id: frame.id,
      error: {
        code: `mcp_error_${json.error.code}`,
        message: json.error.message,
      },
    }
  }
  return { type: 'rpc.response', v: 1, id: frame.id, result: json.result }
}

function errorFrame(id: string, code: string, err: unknown): RpcResponseFrame {
  return {
    type: 'rpc.response',
    v: 1,
    id,
    error: {
      code,
      message: err instanceof Error ? err.message : String(err),
    },
  }
}
