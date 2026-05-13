import type { UIMessage } from 'ai'
import type { SlashCommand, SlashCommandContext, SlashCommandResult } from './types'

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

const registry = new Map<string, SlashCommand>()

/** Register a command (name is lowercased). */
export function registerCommand(cmd: SlashCommand): void {
  registry.set(cmd.name.toLowerCase(), cmd)
}

/** Look up a command by name (case-insensitive). */
export function getCommand(name: string): SlashCommand | undefined {
  return registry.get(name.toLowerCase())
}

/** Return all registered commands. */
export function getAllCommands(): SlashCommand[] {
  return Array.from(registry.values())
}

/** Remove all registered commands (useful for tests). */
export function clearCommands(): void {
  registry.clear()
}

// ---------------------------------------------------------------------------
// Input processing
// ---------------------------------------------------------------------------

/**
 * Regex that matches `/command-name` followed by optional whitespace and
 * arbitrary args (including multiline). Uses `[\s\S]*` instead of `.*` so
 * newlines are captured — this was a critical finding from V3 verification.
 */
const SLASH_COMMAND_RE = /^\/([a-zA-Z0-9_-]+)(?:\s+([\s\S]*))?$/

export interface ProcessSlashCommandDeps {
  messages: UIMessage[]
  conversationId: string
  setMessages: (messages: UIMessage[]) => void
  resetConversation: () => void
  mode: 'chat' | 'agent'
  setMode: (mode: 'chat' | 'agent') => void
}

/**
 * Process raw user input. If it starts with `/` and matches a registered
 * command, delegates to that command's `execute`. Otherwise returns
 * `{ type: 'passthrough', text: input }`.
 */
export function processSlashCommand(
  input: string,
  deps: ProcessSlashCommandDeps,
): SlashCommandResult | Promise<SlashCommandResult> {
  const match = input.match(SLASH_COMMAND_RE)
  if (!match) return { type: 'passthrough', text: input }

  const [, rawName, rawArgs] = match
  const cmd = getCommand(rawName)
  if (!cmd) return { type: 'passthrough', text: input }

  const ctx: SlashCommandContext = {
    args: rawArgs ?? '',
    messages: deps.messages,
    conversationId: deps.conversationId,
    setMessages: deps.setMessages,
    resetConversation: deps.resetConversation,
    mode: deps.mode,
    setMode: deps.setMode,
  }

  return cmd.execute(ctx)
}
