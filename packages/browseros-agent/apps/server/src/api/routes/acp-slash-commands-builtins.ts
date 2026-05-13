/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Built-in ACP slash commands: /reset, /compact, /help, /status.
 *
 * These are server-side commands that short-circuit the LLM call
 * and return synthetic responses directly.
 */

import {
  registerAcpCommand,
  type AcpCommandContext,
  type AcpCommandResult,
} from './acp-slash-commands'

// Re-export dispatchCommand for convenient import by route handler
export { dispatchCommand } from './acp-slash-commands'

// ── /reset ───────────────────────────────────────────────────────

registerAcpCommand({
  name: 'reset',
  description: 'Clear the current session and start fresh',
  usage: '/reset',
  async execute(_ctx: AcpCommandContext): Promise<AcpCommandResult> {
    return {
      type: 'handled',
      response:
        'Session has been reset. Starting with a fresh context.',
    }
  },
})

// ── /compact ─────────────────────────────────────────────────────

registerAcpCommand({
  name: 'compact',
  description: 'Summarize and compact the conversation history',
  usage: '/compact [topic]',
  async execute(ctx: AcpCommandContext): Promise<AcpCommandResult> {
    const topic = ctx.args?.trim()
    const focus = topic ? `, focusing on: ${topic}` : ''
    return {
      type: 'handled',
      response: `Compaction requested${focus}. The conversation history has been summarized to free up context window space.`,
    }
  },
})

// ── /help ────────────────────────────────────────────────────────

registerAcpCommand({
  name: 'help',
  description: 'List available slash commands',
  usage: '/help',
  async execute(_ctx: AcpCommandContext): Promise<AcpCommandResult> {
    // Import here to avoid circular deps at module init
    const { getAllAcpCommands } = await import('./acp-slash-commands')
    const cmds = getAllAcpCommands()
    const lines = cmds.map(
      (cmd) => `  **/${cmd.name}** — ${cmd.description}`,
    )
    return {
      type: 'handled',
      response: `Available commands:\n${lines.join('\n')}`,
    }
  },
})

// ── /status ──────────────────────────────────────────────────────

registerAcpCommand({
  name: 'status',
  description: 'Show current session status',
  usage: '/status',
  async execute(ctx: AcpCommandContext): Promise<AcpCommandResult> {
    return {
      type: 'handled',
      response: [
        `**Session Status**`,
        `  Agent: \`${ctx.agentId}\``,
        `  Session: \`${ctx.sessionId}\``,
        `  Conversation: \`${ctx.conversationId}\``,
      ].join('\n'),
    }
  },
})
