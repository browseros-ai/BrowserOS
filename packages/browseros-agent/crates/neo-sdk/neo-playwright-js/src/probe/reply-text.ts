/**
 * Owns the by-value probe exit into CDP. One JSON string keeps page-owned text and deep trees
 * out of the shared transport's JSON reader; Rust checks the same nesting bound before parsing.
 * Node-handle replies bypass this codec and retain their remote identity.
 */
import { MAX_REPLY_DEPTH } from './generated'

const FALLBACK =
  '{"kind":"error","class":"internal","message":"Neo could not serialize the probe reply."}'

/** JSON nesting outside strings, using the same byte-independent scan as page_value.rs. */
export function replyDepth(text: string): number {
  let depth = 0
  let deepest = 0
  let quoted = false
  let escaped = false
  for (let index = 0; index < text.length; index++) {
    const char = text[index]
    if (quoted) {
      if (escaped) escaped = false
      else if (char === '\\') escaped = true
      else if (char === '"') quoted = false
    } else if (char === '"') quoted = true
    else if (char === '[' || char === '{') deepest = Math.max(deepest, ++depth)
    else if (char === ']' || char === '}') depth = Math.max(0, depth - 1)
  }
  return deepest
}

/**
 * Native JSON.stringify escapes lone UTF-16 halves and emits intact pairs unchanged. Consume
 * an escaped backslash first so a page's literal backslash-u text remains literal.
 */
export function wellFormedReply(text: string): string {
  return text.replace(/\\\\|\\u[dD][89a-fA-F][0-9a-fA-F]{2}/g, (token) =>
    token === '\\\\' ? token : '\ufffd',
  )
}

function depthError(depth: number): string {
  return JSON.stringify({
    kind: 'error',
    class: 'unsupported',
    message: `The snapshot reply reaches JSON depth ${depth}, above the limit of ${MAX_REPLY_DEPTH}. Pass { depth: n } or take a locator's snapshot of a section.`,
  })
}

/** Never throws: even serialization failures cross as a small fixed, readable error. */
export function replyText(reply: unknown): string {
  try {
    let text = JSON.stringify(reply)
    if (text === undefined) return FALLBACK
    let depth = replyDepth(text)
    if (depth > MAX_REPLY_DEPTH) {
      // A diagnostic cover tree is optional; its other facts still explain a refused click.
      if (
        typeof reply === 'object' &&
        reply !== null &&
        'kind' in reply &&
        reply.kind === 'cover' &&
        'facts' in reply &&
        typeof reply.facts === 'object' &&
        reply.facts !== null &&
        'snapshot' in reply.facts
      ) {
        text = JSON.stringify({
          ...reply,
          facts: { ...reply.facts, snapshot: null },
        })
        depth = replyDepth(text)
      }
      if (depth > MAX_REPLY_DEPTH) return depthError(depth)
    }
    return wellFormedReply(text)
  } catch {
    return FALLBACK
  }
}
