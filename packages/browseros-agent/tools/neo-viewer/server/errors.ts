/**
 * An error as one block of text for the terminal. The page build fails with an AggregateError
 * whose message is only "the web page did not build"; what the user needs is inside: each
 * bundler message with its file and line ("Could not resolve …, maybe you need to bun install").
 */

interface Position {
  file?: string
  line?: number
  column?: number
}

export function describeError(error: unknown): string {
  if (!(error instanceof Error)) return String(error)
  const lines = [error.message]
  if (error instanceof AggregateError) {
    for (const inner of error.errors) lines.push(`  ${describeInner(inner)}`)
  }
  return lines.join('\n')
}

/** `web/main.tsx:7:8: Could not resolve: "…"` for a bundler message; the message otherwise. */
function describeInner(inner: unknown): string {
  if (typeof inner !== 'object' || inner === null) return String(inner)
  const { message, position } = inner as {
    message?: unknown
    position?: Position | null
  }
  const text = typeof message === 'string' ? message : String(inner)
  if (!position?.file) return text
  const at = [position.file, position.line, position.column].filter(
    (part) => part !== undefined && part !== null,
  )
  return `${at.join(':')}: ${text}`
}
