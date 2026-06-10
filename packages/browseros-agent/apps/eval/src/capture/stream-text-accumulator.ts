import {
  type UIMessageStreamEvent,
  UIMessageStreamEventSchema,
} from '@browseros/shared/schemas/ui-stream'

/** Parse SSE data lines into validated UIMessageStreamEvents. */
export function parseSSEEvents(data: string): UIMessageStreamEvent[] {
  const events: UIMessageStreamEvent[] = []
  const lines = data.split('\n')
  for (const line of lines) {
    if (!line.startsWith('data: ')) continue
    const jsonStr = line.slice(6)
    if (jsonStr === '[DONE]') continue
    try {
      const parsed = JSON.parse(jsonStr)
      const result = UIMessageStreamEventSchema.safeParse(parsed)
      if (result.success) events.push(result.data)
    } catch {
      // Ignore parse errors
    }
  }
  return events
}
