/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * ACP slash command registry and dispatch layer.
 *
 * Commands are registered server-side and dispatched BEFORE
 * `service.startTurn()` so that built-in commands can short-circuit
 * the LLM call entirely (e.g. /reset, /help, /status).
 *
 * ACP mode REJECTS unknown commands (unlike LLM-mode which passes
 * them through as regular text).
 */

/**
 * Mirrors ACP SDK `AvailableCommand` from
 * `@agentclientprotocol/sdk/dist/schema/types.gen.d.ts`.
 * Not re-exported from the SDK's main entry, so we define locally.
 */
export interface AvailableCommand {
  name: string
  description: string
}

// ── Types ────────────────────────────────────────────────────────

export interface AcpCommandResult {
  type: 'handled' | 'passthrough' | 'error'
  /** For handled commands, the response text streamed back to the client */
  response?: string
  /** For error type, the error message */
  error?: string
}

export interface AcpCommandContext {
  agentId: string
  conversationId: string
  sessionId: string
  args: string
}

export interface AcpServerCommand {
  name: string
  description: string
  usage: string
  execute: (ctx: AcpCommandContext) => Promise<AcpCommandResult>
}

// ── Registry ─────────────────────────────────────────────────────

const commands = new Map<string, AcpServerCommand>()

/** Register a server-side ACP command */
export function registerAcpCommand(cmd: AcpServerCommand): void {
  commands.set(cmd.name, cmd)
}

/** Look up a command by name */
export function getAcpCommand(
  name: string,
): AcpServerCommand | undefined {
  return commands.get(name)
}

/** Get all registered commands */
export function getAllAcpCommands(): AcpServerCommand[] {
  return Array.from(commands.values())
}

/** Clear all registered commands (useful for tests) */
export function clearAcpCommands(): void {
  commands.clear()
}

// ── Dispatch ─────────────────────────────────────────────────────

const SLASH_COMMAND_RE = /^\/([a-zA-Z0-9_-]+)(?:\s+([\s\S]*))?$/

/**
 * Dispatch a slash command. Returns:
 * - `{ type: 'handled' }` — command executed, response ready
 * - `{ type: 'error' }` — unknown command in ACP mode
 * - `{ type: 'passthrough' }` — not a slash command, proceed to LLM
 */
export async function dispatchCommand(
  input: string,
  ctx: AcpCommandContext,
): Promise<AcpCommandResult> {
  const match = input.match(SLASH_COMMAND_RE)
  if (!match) return { type: 'passthrough' }

  const [, cmdName, args] = match
  const cmd = getAcpCommand(cmdName)
  if (!cmd) {
    return {
      type: 'error',
      error: `Unknown command: /${cmdName}. Available: ${Array.from(commands.keys())
        .map((k) => '/' + k)
        .join(', ')}`,
    }
  }

  return cmd.execute({ ...ctx, args: args ?? '' })
}

// ── ACP SDK format ───────────────────────────────────────────────

/**
 * Convert server commands to ACP `AvailableCommand[]` format for
 * inclusion in session capabilities / AvailableCommandsUpdate events.
 */
export function toAvailableCommands(): AvailableCommand[] {
  return Array.from(commands.values()).map((cmd) => ({
    name: cmd.name,
    description: cmd.description,
  }))
}
