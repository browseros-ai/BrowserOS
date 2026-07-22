import type { UIMessage } from 'ai'

type UIPart = UIMessage['parts'][number]

export const OMITTED_IMAGE_TEXT = '[image omitted to conserve memory]'

interface ToolResultContentBlock {
  type?: string
  text?: string
  data?: string
  mimeType?: string
}

interface ToolResultOutput {
  content?: ToolResultContentBlock[]
}

function isToolPart(part: UIPart): boolean {
  return part.type.startsWith('tool-') || part.type === 'dynamic-tool'
}

function stripImagesFromOutput(output: unknown): unknown {
  if (!output || typeof output !== 'object') return output
  const content = (output as ToolResultOutput).content
  if (!Array.isArray(content)) return output
  let changed = false
  const nextContent = content.map((block) => {
    if (block?.type === 'image' && typeof block.data === 'string') {
      changed = true
      return { type: 'text', text: OMITTED_IMAGE_TEXT }
    }
    return block
  })
  return changed
    ? { ...(output as ToolResultOutput), content: nextContent }
    : output
}

function stripPart(part: UIPart): UIPart {
  if (!isToolPart(part)) return part
  const output = (part as { output?: unknown }).output
  if (output === undefined) return part
  const nextOutput = stripImagesFromOutput(output)
  if (nextOutput === output) return part
  return { ...part, output: nextOutput } as UIPart
}

function stripMessage(message: UIMessage): UIMessage {
  if (!message.parts?.length) return message
  let changed = false
  const parts = message.parts.map((part) => {
    const next = stripPart(part)
    if (next !== part) changed = true
    return next
  })
  return changed ? { ...message, parts } : message
}

/**
 * Replace retained base64 image blocks in tool-result outputs with a small
 * text marker. Nothing in the side panel renders these images, but the AI
 * SDK keeps every message resident for the whole session, so the screenshots
 * pile up until the renderer runs out of memory (issue #1972). Returns the
 * same array reference when there is nothing to strip so callers can skip a
 * redundant state update. `keepLastMessage` leaves the most recent message
 * untouched, used for in-memory trimming where the latest turn stays live.
 */
export function stripImageToolOutputs(
  messages: UIMessage[],
  options?: { keepLastMessage?: boolean },
): UIMessage[] {
  const keepLastIndex = options?.keepLastMessage ? messages.length - 1 : -1
  let changed = false
  const next = messages.map((message, index) => {
    if (index === keepLastIndex) return message
    const stripped = stripMessage(message)
    if (stripped !== message) changed = true
    return stripped
  })
  return changed ? next : messages
}
