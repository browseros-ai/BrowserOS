import type {
  LanguageModelV2FinishReason,
  LanguageModelV2StreamPart,
  LanguageModelV2Usage,
} from '@ai-sdk/provider'
import type { AcpRuntimeEvent, AcpRuntimeTurnResult } from 'acpx/runtime'
import { fromRuntimeError } from './errors.ts'

type TextStream = 'output' | 'thought'
type BlockKind = TextStream | null

interface ToolCallState {
  blockId: string
  toolName: string
  emittedText: string
  inputClosed: boolean
}

export interface EventTranslatorOptions {
  generateId: () => string
}

export interface FinishOptions {
  result: AcpRuntimeTurnResult
  /** Optional usage override; defaults to whatever was accumulated from `status` events. */
  usage?: LanguageModelV2Usage
}

const EMPTY_USAGE: LanguageModelV2Usage = {
  inputTokens: undefined,
  outputTokens: undefined,
  totalTokens: undefined,
}

const STOP_REASON_MAP: Record<string, LanguageModelV2FinishReason> = {
  end_turn: 'stop',
  stop_sequence: 'stop',
  max_tokens: 'length',
  tool_calls: 'tool-calls',
  tool_use: 'tool-calls',
}

/**
 * Translates `AcpRuntimeEvent`s into AI SDK V2 stream parts.
 *
 * Holds state between `translate()` calls so that text/reasoning blocks
 * open exactly once and tool input deltas are diffed against previously
 * emitted text. Caller is expected to:
 *
 *  1. Call `translate(event)` for each runtime event, enqueuing the
 *     returned parts.
 *  2. After the runtime turn ends (via `turn.events` iterator
 *     completion), call `flush()` to close any open block and then
 *     `finish(result)` to emit the terminal `finish` part.
 */
export class EventTranslator {
  private readonly generateId: () => string
  private currentBlock: BlockKind = null
  private currentBlockId: string | null = null
  private readonly toolCalls = new Map<string, ToolCallState>()
  private accumulatedTotalTokens: number | undefined
  private accumulatedSize: number | undefined

  constructor(opts: EventTranslatorOptions) {
    this.generateId = opts.generateId
  }

  translate(event: AcpRuntimeEvent): LanguageModelV2StreamPart[] {
    switch (event.type) {
      case 'text_delta':
        return this.handleTextDelta(event)
      case 'tool_call':
        return this.handleToolCall(event)
      case 'status':
        return this.handleStatus(event)
      case 'error':
        return this.handleError(event)
      case 'done':
        return []
    }
  }

  flush(): LanguageModelV2StreamPart[] {
    const parts: LanguageModelV2StreamPart[] = []
    parts.push(...this.closeCurrentBlock())
    for (const [, state] of this.toolCalls) {
      if (!state.inputClosed) {
        parts.push({ type: 'tool-input-end', id: state.blockId })
        state.inputClosed = true
      }
    }
    return parts
  }

  finish(opts: FinishOptions): LanguageModelV2StreamPart {
    const { result } = opts
    const finishReason = mapFinishReason(result)
    const usage = opts.usage ?? this.accumulatedUsage()

    const part: Extract<LanguageModelV2StreamPart, { type: 'finish' }> = {
      type: 'finish',
      finishReason,
      usage,
    }
    if (result.status === 'failed') {
      part.providerMetadata = {
        acpx: {
          errorCode: result.error.code ?? 'unknown',
          errorMessage: result.error.message,
        },
      }
    }
    return part
  }

  errorPartIfFailed(result: AcpRuntimeTurnResult): LanguageModelV2StreamPart[] {
    if (result.status !== 'failed') return []
    return [{ type: 'error', error: fromRuntimeError(result.error) }]
  }

  private accumulatedUsage(): LanguageModelV2Usage {
    if (
      this.accumulatedTotalTokens === undefined &&
      this.accumulatedSize === undefined
    ) {
      return EMPTY_USAGE
    }
    return {
      inputTokens: undefined,
      outputTokens: undefined,
      totalTokens: this.accumulatedTotalTokens,
      cachedInputTokens: this.accumulatedSize,
    }
  }

  private handleTextDelta(
    event: Extract<AcpRuntimeEvent, { type: 'text_delta' }>,
  ): LanguageModelV2StreamPart[] {
    const target: TextStream = event.stream === 'thought' ? 'thought' : 'output'
    const parts: LanguageModelV2StreamPart[] = []

    if (this.currentBlock !== target) {
      parts.push(...this.closeCurrentBlock())
      const id = this.generateId()
      parts.push({
        type: target === 'thought' ? 'reasoning-start' : 'text-start',
        id,
      })
      this.currentBlock = target
      this.currentBlockId = id
    }

    if (event.text.length > 0 && this.currentBlockId) {
      parts.push({
        type: target === 'thought' ? 'reasoning-delta' : 'text-delta',
        id: this.currentBlockId,
        delta: event.text,
      })
    }

    return parts
  }

  private closeCurrentBlock(): LanguageModelV2StreamPart[] {
    if (!this.currentBlock || !this.currentBlockId) return []
    const part: LanguageModelV2StreamPart = {
      type: this.currentBlock === 'thought' ? 'reasoning-end' : 'text-end',
      id: this.currentBlockId,
    }
    this.currentBlock = null
    this.currentBlockId = null
    return [part]
  }

  private handleToolCall(
    event: Extract<AcpRuntimeEvent, { type: 'tool_call' }>,
  ): LanguageModelV2StreamPart[] {
    const callId = event.toolCallId
    if (!callId) return []

    const parts: LanguageModelV2StreamPart[] = []
    parts.push(...this.closeCurrentBlock())

    let state = this.toolCalls.get(callId)
    if (!state) {
      const blockId = this.generateId()
      const toolName = event.title?.trim() || 'tool'
      state = { blockId, toolName, emittedText: '', inputClosed: false }
      this.toolCalls.set(callId, state)
      parts.push({ type: 'tool-input-start', id: blockId, toolName })
    }

    parts.push(...this.appendToolText(state, event.text))

    if (isTerminalToolStatus(event.status)) {
      parts.push(
        ...this.finalizeToolCall(callId, state, event.status === 'failed'),
      )
    }
    return parts
  }

  private appendToolText(
    state: ToolCallState,
    text: string,
  ): LanguageModelV2StreamPart[] {
    if (!text) return []

    let delta = ''
    if (text.startsWith(state.emittedText)) {
      delta = text.slice(state.emittedText.length)
    } else {
      delta = text
    }
    if (!delta) return []

    state.emittedText += delta
    return [{ type: 'tool-input-delta', id: state.blockId, delta }]
  }

  private finalizeToolCall(
    callId: string,
    state: ToolCallState,
    failed: boolean,
  ): LanguageModelV2StreamPart[] {
    const parts: LanguageModelV2StreamPart[] = []
    if (!state.inputClosed) {
      parts.push({ type: 'tool-input-end', id: state.blockId })
      state.inputClosed = true
    }
    parts.push({
      type: 'tool-call',
      toolCallId: callId,
      toolName: state.toolName,
      input: state.emittedText,
      providerExecuted: true,
    })
    parts.push({
      type: 'tool-result',
      toolCallId: callId,
      toolName: state.toolName,
      result: state.emittedText,
      ...(failed ? { isError: true } : {}),
    })
    this.toolCalls.delete(callId)
    return parts
  }

  private handleStatus(
    event: Extract<AcpRuntimeEvent, { type: 'status' }>,
  ): LanguageModelV2StreamPart[] {
    if (event.tag === 'usage_update') {
      if (event.used !== undefined) this.accumulatedTotalTokens = event.used
      if (event.size !== undefined) this.accumulatedSize = event.size
      return []
    }
    if (event.tag === 'plan') {
      // Emit plan as a self-contained reasoning block with its own id.
      // Doesn't disturb the currently-open text/reasoning block — AI SDK
      // allows multiple reasoning ids to coexist as long as each is
      // properly start/end-paired. The `[Plan]` prefix lets consumers
      // distinguish plan announcements from the agent's chain-of-thought.
      // Trim because whitespace-only plan updates carry no signal — same
      // intent as the empty-string case.
      const trimmed = event.text.trim()
      if (trimmed.length === 0) return []
      const id = this.generateId()
      return [
        { type: 'reasoning-start', id },
        { type: 'reasoning-delta', id, delta: `[Plan] ${trimmed}` },
        { type: 'reasoning-end', id },
      ]
    }
    return []
  }

  private handleError(
    event: Extract<AcpRuntimeEvent, { type: 'error' }>,
  ): LanguageModelV2StreamPart[] {
    const error = fromRuntimeError({
      message: event.message,
      code: event.code,
      retryable: event.retryable,
    })
    return [{ type: 'error', error }]
  }
}

function isTerminalToolStatus(status: string | undefined): boolean {
  return status === 'completed' || status === 'failed'
}

function mapFinishReason(
  result: AcpRuntimeTurnResult,
): LanguageModelV2FinishReason {
  if (result.status === 'cancelled') return 'other'
  if (result.status === 'failed') return 'error'
  if (!result.stopReason) return 'stop'
  return STOP_REASON_MAP[result.stopReason] ?? 'unknown'
}
