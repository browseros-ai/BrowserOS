import type { SlashCommand, SlashCommandResult } from './types'
import { getAllCommands, registerCommand } from './registry'

// ---------------------------------------------------------------------------
// /new — Reset conversation
// ---------------------------------------------------------------------------

const newCommand: SlashCommand = {
  name: 'new',
  description: 'Start a new conversation',
  usage: '/new',
  type: 'action',
  allowDuringBusy: true,
  execute(ctx) {
    ctx.resetConversation()
    return { type: 'action', handled: true }
  },
}

// ---------------------------------------------------------------------------
// /compact — Summarize conversation
// ---------------------------------------------------------------------------

const compactCommand: SlashCommand = {
  name: 'compact',
  description: 'Summarize the conversation so far',
  usage: '/compact [topic to focus on]',
  type: 'prompt',
  execute(ctx) {
    const topic = ctx.args.trim()
    const focus = topic ? `, focusing on: ${topic}` : ''
    return {
      type: 'prompt',
      expandedText: `Summarize our conversation so far concisely${focus}. Highlight key decisions, actions taken, and open items.`,
    }
  },
}

// ---------------------------------------------------------------------------
// /help — Show available commands
// ---------------------------------------------------------------------------

const helpCommand: SlashCommand = {
  name: 'help',
  description: 'Show available slash commands',
  usage: '/help',
  type: 'action',
  execute() {
    const commands = getAllCommands()
    const lines = commands.map(
      (cmd) => `**/${cmd.name}** — ${cmd.description}\n  Usage: \`${cmd.usage}\``,
    )
    const helpText = [
      '## Available Commands',
      '',
      ...lines,
      '',
      'Type any unknown `/command` to send it as regular text.',
    ].join('\n')

    // Return as prompt so the LLM renders it nicely
    return { type: 'prompt', expandedText: helpText }
  },
}

// ---------------------------------------------------------------------------
// /mode — Switch chat mode
// ---------------------------------------------------------------------------

const modeCommand: SlashCommand = {
  name: 'mode',
  description: 'Switch between chat and agent mode',
  usage: '/mode <chat|agent>',
  type: 'action',
  execute(ctx): SlashCommandResult {
    const arg = ctx.args.trim().toLowerCase()
    if (arg === 'chat' || arg === 'agent') {
      ctx.setMode(arg as 'chat' | 'agent')
      return { type: 'action', handled: true }
    }
    return {
      type: 'prompt',
      expandedText: `Current mode is **${ctx.mode}**. Use \`/mode chat\` or \`/mode agent\` to switch.`,
    }
  },
}

// ---------------------------------------------------------------------------
// /fork — Fork conversation (stub — delegates to undo-fork feature)
// ---------------------------------------------------------------------------

const forkCommand: SlashCommand = {
  name: 'fork',
  description: 'Fork the conversation from a previous point',
  usage: '/fork',
  type: 'action',
  execute() {
    // Stub: the actual fork logic is handled by the undo-fork feature.
    // This command signals intent; UI will show fork UI on the messages.
    return {
      type: 'prompt',
      expandedText:
        'To fork a conversation, hover over any user message and click the ⑂ fork button. This creates a new conversation with all messages up to that point.',
    }
  },
}

// ---------------------------------------------------------------------------
// /undo — Undo last turn (stub — delegates to undo-fork feature)
// ---------------------------------------------------------------------------

const undoCommand: SlashCommand = {
  name: 'undo',
  description: 'Undo the last turn in the conversation',
  usage: '/undo',
  type: 'action',
  execute() {
    // Stub: the actual undo logic is handled by the undo-fork feature.
    return {
      type: 'prompt',
      expandedText:
        'To undo a turn, hover over any user message and click the ↩ undo button. This removes that turn and everything after it.',
    }
  },
}

// ---------------------------------------------------------------------------
// Register all built-in commands
// ---------------------------------------------------------------------------

export function registerBuiltinCommands(): void {
  registerCommand(newCommand)
  registerCommand(compactCommand)
  registerCommand(helpCommand)
  registerCommand(modeCommand)
  registerCommand(forkCommand)
  registerCommand(undoCommand)
}

/** Re-export for convenience */
export { newCommand, compactCommand, helpCommand, modeCommand, forkCommand, undoCommand }
