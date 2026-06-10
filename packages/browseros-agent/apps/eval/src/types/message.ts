import {
  type UIMessageStreamEvent,
  UIMessageStreamEventSchema,
} from '@browseros/shared/schemas/ui-stream'
import { z } from 'zod'

// ============================================================================
// Eval-specific message types (not in UIMessageStreamEvent)
// ============================================================================

// User's initial query
const UserMessageSchema = z.object({
  type: z.literal('user'),
  timestamp: z.string(),
  content: z.string(),
})

// ============================================================================
// Stream event with eval additions (timestamp, screenshot)
// ============================================================================

const EvalStreamEventSchema = UIMessageStreamEventSchema.and(
  z.object({
    timestamp: z.string(),
    screenshot: z.number().optional(),
  }),
)

// ============================================================================
// Message Schema (union of all message types)
// ============================================================================

export const MessageSchema = z.union([UserMessageSchema, EvalStreamEventSchema])

// ============================================================================
// Type Exports
// ============================================================================

export type UserMessage = z.infer<typeof UserMessageSchema>
export type EvalStreamEvent = z.infer<typeof EvalStreamEventSchema>
export type Message = z.infer<typeof MessageSchema>
export type { UIMessageStreamEvent }

// ============================================================================
// Type Guards for Stream Events
// ============================================================================

/** Check if message is a text-start event */
function isTextStart(
  msg: Message,
): msg is EvalStreamEvent & { type: 'text-start' } {
  return msg.type === 'text-start'
}

/** Check if message is a text-delta event */
function isTextDelta(
  msg: Message,
): msg is EvalStreamEvent & { type: 'text-delta'; delta: string } {
  return msg.type === 'text-delta'
}

/** Check if message is a text-end event */
function isTextEnd(
  msg: Message,
): msg is EvalStreamEvent & { type: 'text-end' } {
  return msg.type === 'text-end'
}

/** Check if message is a tool-input-available event */
export function isToolInputAvailable(msg: Message): msg is EvalStreamEvent & {
  type: 'tool-input-available'
  toolCallId: string
  toolName: string
  input: unknown
} {
  return msg.type === 'tool-input-available'
}

/** Check if message is a tool-output-error event */
export function isToolOutputError(msg: Message): msg is EvalStreamEvent & {
  type: 'tool-output-error'
  toolCallId: string
  error: unknown
} {
  return msg.type === 'tool-output-error'
}

/** Check if message is a tool-input-error event */
export function isToolInputError(msg: Message): msg is EvalStreamEvent & {
  type: 'tool-input-error'
  toolCallId: string
  error: unknown
} {
  return msg.type === 'tool-input-error'
}

// ============================================================================
// Helper Functions for Message Processing
// ============================================================================

/**
 * Extract the last complete assistant text from messages.
 * Accumulates text-delta events between text-start and text-end.
 */
export function extractLastAssistantText(messages: Message[]): string | null {
  let lastText = ''
  let currentText = ''

  for (const msg of messages) {
    if (isTextStart(msg)) {
      currentText = ''
    } else if (isTextDelta(msg)) {
      currentText += msg.delta
    } else if (isTextEnd(msg)) {
      lastText = currentText
      currentText = ''
    }
  }

  return lastText || null
}
