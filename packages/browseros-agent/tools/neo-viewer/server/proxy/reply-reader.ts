/**
 * Reads Neo's reply as it streams past, for the log only: the proxy forwards each chunk before
 * handing it here, and nothing here can change what the agent receives.
 *
 * Neo answers every POST as Server-Sent Events (`text/event-stream`): `data:` lines carry
 * JSON-RPC messages (notifications first, the response last) and `:` comment lines are its
 * keep-alive every 5 s while a run is going. A plain `application/json` body is read whole.
 */

/** One JSON-RPC message seen in the reply, with when it went past. */
export interface ReplyMessage {
  message: unknown
  at: number
}

export interface ReplyListener {
  message(message: ReplyMessage): void
  comment(at: number): void
}

/** The WHATWG event-stream parser, reduced to what MCP uses: `data` lines and comments. */
export class SseReader {
  private readonly decoder = new TextDecoder()
  private buffer = ''
  private data: string[] = []
  private skipLeadingLf = false

  constructor(private readonly listener: ReplyListener) {}

  push(chunk: Uint8Array, at: number): void {
    this.feed(this.decoder.decode(chunk, { stream: true }), at)
  }

  end(at: number): void {
    this.feed(this.decoder.decode(), at)
    // A stream cut mid-event dispatches nothing, as EventSource does.
  }

  private feed(text: string, at: number): void {
    this.buffer += text
    let start = 0
    for (let i = 0; i < this.buffer.length; i++) {
      const char = this.buffer[i]
      if (char === '\n' && this.skipLeadingLf && i === start) {
        // The LF of a CRLF split across chunks.
        this.skipLeadingLf = false
        start = i + 1
        continue
      }
      this.skipLeadingLf = false
      if (char !== '\n' && char !== '\r') continue
      this.line(this.buffer.slice(start, i), at)
      if (char === '\r') {
        if (this.buffer[i + 1] === '\n') i++
        else if (i + 1 === this.buffer.length) this.skipLeadingLf = true
      }
      start = i + 1
    }
    this.buffer = this.buffer.slice(start)
  }

  private line(line: string, at: number): void {
    if (line === '') {
      if (this.data.length > 0) this.dispatch(this.data.join('\n'), at)
      this.data = []
      return
    }
    if (line.startsWith(':')) {
      this.listener.comment(at)
      return
    }
    const colon = line.indexOf(':')
    const field = colon === -1 ? line : line.slice(0, colon)
    if (field !== 'data') return
    let value = colon === -1 ? '' : line.slice(colon + 1)
    if (value.startsWith(' ')) value = value.slice(1)
    this.data.push(value)
  }

  private dispatch(data: string, at: number): void {
    try {
      this.listener.message({ message: JSON.parse(data), at })
    } catch {
      // Not JSON-RPC; the raw body still has it.
    }
  }
}

/** An `application/json` reply: one message or a batch, read when the body ends. */
export class JsonReader {
  private readonly chunks: Uint8Array[] = []

  constructor(private readonly listener: ReplyListener) {}

  push(chunk: Uint8Array, _at: number): void {
    this.chunks.push(chunk)
  }

  end(at: number): void {
    let parsed: unknown
    try {
      parsed = JSON.parse(new TextDecoder().decode(Buffer.concat(this.chunks)))
    } catch {
      return
    }
    for (const message of Array.isArray(parsed) ? parsed : [parsed]) {
      this.listener.message({ message, at })
    }
  }
}
