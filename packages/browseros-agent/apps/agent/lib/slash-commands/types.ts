import type { UIMessage } from 'ai'

/**
 * A registered slash command.
 * @public
 */
export interface SlashCommand {
  /** Command name without slash, e.g. 'compact' */
  name: string
  /** Short description shown in autocomplete */
  description: string
  /** Usage hint, e.g. '/compact [topic]' */
  usage: string
  /** Whether this is a prompt-expansion, a side-effecting action, or a file-based template */
  type: 'prompt' | 'action' | 'template'
  /** If true, can execute while the AI is streaming */
  allowDuringBusy?: boolean
  /** Execute the command */
  execute: (ctx: SlashCommandContext) => SlashCommandResult | Promise<SlashCommandResult>
}

/**
 * Context passed to every slash command execution.
 * @public
 */
export interface SlashCommandContext {
  /** The raw argument string after the command name */
  args: string
  /** Current conversation messages */
  messages: UIMessage[]
  /** Current conversation ID */
  conversationId: string
  /** Replace the entire message array */
  setMessages: (messages: UIMessage[]) => void
  /** Generate a new conversation ID (for /new) */
  resetConversation: () => void
  /** Current chat mode */
  mode: 'chat' | 'agent'
  /** Switch chat mode */
  setMode: (mode: 'chat' | 'agent') => void
}

/**
 * Result of a slash command execution.
 * @public
 */
export type SlashCommandResult =
  | { type: 'prompt'; expandedText: string }
  | { type: 'action'; handled: true }
  | { type: 'passthrough'; text: string }
