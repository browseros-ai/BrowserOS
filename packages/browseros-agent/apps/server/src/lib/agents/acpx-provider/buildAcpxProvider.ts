/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { homedir } from 'node:os'
import { join } from 'node:path'
import {
  type AcpPermissionDecision,
  type AcpPermissionRequest,
  type AcpxMcpServerConfig,
  type AcpxNonInteractivePermissions,
  type AcpxPermissionMode,
  type AcpxProvider,
  createAcpxProvider,
} from 'acpx-ai-provider'
import { getBrowserosDir } from '../../browseros-dir'

/**
 * Storage-side MCP shape: `{name, value}` pair arrays match the
 * extension's existing settings layout. The wire format `acpx-ai-provider`
 * wants is a flat `Record<string, string>` for env / headers, so we
 * convert at the factory boundary via `toProviderShape` below.
 */
export interface McpServerStdio {
  type: 'stdio'
  name: string
  command: string
  args: string[]
  env: Array<{ name: string; value: string }>
}

export interface McpServerHttp {
  type: 'http' | 'sse'
  name: string
  url: string
  headers: Array<{ name: string; value: string }>
}

export type McpServerSpec = McpServerStdio | McpServerHttp

export type PermissionRequestHandler = (
  req: AcpPermissionRequest,
  ctx: { signal: AbortSignal },
) => Promise<AcpPermissionDecision | undefined>

export interface BuildAcpxProviderOptions {
  /**
   * Identifier for the chat / turn loop the provider is being built
   * for. Drives the default `sessionKey` so concurrent conversations
   * never collide on the same persistent ACP session record.
   */
  conversationId: string
  /**
   * Agent id resolved by acpx's registry. Built-ins ('claude', 'codex',
   * etc.) are looked up directly; user-supplied entries are merged in
   * via `agentRegistryOverrides`.
   */
  agentId: string
  /** Working directory the agent runs against. Defaults to `$HOME`. */
  workspacePath?: string
  /**
   * Override the persistent session key. By default the conversation id
   * is the key so a single conversation always reaches the same ACP
   * session record across reconnects.
   */
  sessionKey?: string
  resumeSessionId?: string | null
  /**
   * MCP servers to expose to the spawned agent. The list is global to
   * the chat today (per-provider scoping is a deliberate follow-up).
   */
  mcpServers?: McpServerSpec[]
  /**
   * Default policy when `onPermissionRequest` is unset or resolves to
   * `undefined`. BrowserOS ships ACP agents in `approve-all` so read +
   * write tools run end-to-end without a UI gate; callers that want a
   * stricter mode pass it explicitly.
   */
  permissionMode?: AcpxPermissionMode
  /**
   * Policy for permission requests when no human is around to answer
   * (CLI / scheduled tasks). 'deny' is the safe default; flip to
   * 'allow' only for trusted background flows.
   */
  nonInteractivePermissions?: AcpxNonInteractivePermissions
  /**
   * Bridge to the renderer's permission UI. Resolve to a decision to
   * gate the call, or to `undefined` to fall back to `permissionMode`.
   * Wiring lands with the chat-route integration in a later phase.
   */
  onPermissionRequest?: PermissionRequestHandler
  /**
   * Extra agent-id → shell-command mappings on top of acpx's built-in
   * registry. Hermes and user-supplied custom agents flow through here.
   * Phase 1 leaves it empty; later phases populate it from settings.
   */
  agentRegistryOverrides?: Record<string, string>
  /**
   * On-disk location for persistent ACP session records. Defaults to a
   * subdirectory under `getBrowserosDir()` so it sits next to the rest
   * of the BrowserOS state and gets cleaned up with the install.
   */
  stateDir?: string
}

const DEFAULT_PERMISSION_MODE: AcpxPermissionMode = 'approve-all'
const DEFAULT_NON_INTERACTIVE_PERMISSIONS: AcpxNonInteractivePermissions =
  'deny'

function defaultStateDir(): string {
  return join(getBrowserosDir(), 'acpx-state')
}

function pairsToRecord(
  pairs: Array<{ name: string; value: string }>,
): Record<string, string> {
  const out: Record<string, string> = {}
  for (const { name, value } of pairs) out[name] = value
  return out
}

function toProviderShape(server: McpServerSpec): AcpxMcpServerConfig {
  if (server.type === 'stdio') {
    return {
      type: 'stdio',
      name: server.name,
      command: server.command,
      args: server.args,
      env: pairsToRecord(server.env),
    }
  }
  return {
    type: server.type,
    name: server.name,
    url: server.url,
    headers: pairsToRecord(server.headers),
  }
}

/**
 * Build an `AcpxProvider` for a given conversation + agent. The returned
 * provider exposes `languageModel()` so the caller can drop it into
 * `streamText({ model })` exactly like the model-backed factories in
 * `provider-factory.ts`. Lifecycle (spawn child, open ACP session,
 * close) is owned by the provider; the caller is responsible for
 * matching `close()` calls when the conversation ends or the provider
 * tuple changes.
 */
export async function buildAcpxProvider(
  opts: BuildAcpxProviderOptions,
): Promise<AcpxProvider> {
  return createAcpxProvider({
    agent: opts.agentId,
    cwd: opts.workspacePath ?? homedir(),
    sessionKey: opts.sessionKey ?? opts.conversationId,
    sessionMode: 'persistent',
    stateDir: opts.stateDir ?? defaultStateDir(),
    resumeSessionId: opts.resumeSessionId ?? undefined,
    agentRegistryOverrides: opts.agentRegistryOverrides ?? {},
    permissionMode: opts.permissionMode ?? DEFAULT_PERMISSION_MODE,
    nonInteractivePermissions:
      opts.nonInteractivePermissions ?? DEFAULT_NON_INTERACTIVE_PERMISSIONS,
    onPermissionRequest: opts.onPermissionRequest,
    mcpServers: opts.mcpServers?.map(toProviderShape),
  })
}

export const __internal__ = {
  pairsToRecord,
  toProviderShape,
  defaultStateDir,
  DEFAULT_PERMISSION_MODE,
  DEFAULT_NON_INTERACTIVE_PERMISSIONS,
}
