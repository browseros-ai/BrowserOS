import type {
  LanguageModelV2,
  LanguageModelV2CallOptions,
  LanguageModelV2CallWarning,
  LanguageModelV2Content,
  LanguageModelV2FinishReason,
  LanguageModelV2StreamPart,
  LanguageModelV2Usage,
  SharedV2ProviderMetadata,
} from '@ai-sdk/provider'
import type { AcpRuntimeTurn } from 'acpx/runtime'
import { EventTranslator } from './convert-events.ts'
import { convertPrompt } from './convert-prompt.ts'
import { createJsonCleanupTransform } from './json-output.ts'
import type { AcpxProvider } from './provider.ts'
import type { AcpxLanguageModelOptions } from './types.ts'

const DEFAULT_TURN_TIMEOUT_MS = 5 * 60_000

const EMPTY_USAGE: LanguageModelV2Usage = {
  inputTokens: undefined,
  outputTokens: undefined,
  totalTokens: undefined,
}

export interface DoStreamResult {
  stream: ReadableStream<LanguageModelV2StreamPart>
  request: { body: unknown }
  response: { headers: Record<string, string> }
}

export interface DoGenerateResult {
  content: LanguageModelV2Content[]
  finishReason: LanguageModelV2FinishReason
  usage: LanguageModelV2Usage
  providerMetadata?: SharedV2ProviderMetadata
  request: { body: unknown }
  response: { headers: Record<string, string> }
  warnings: LanguageModelV2CallWarning[]
}

export class AcpxLanguageModel implements LanguageModelV2 {
  readonly specificationVersion = 'v2'
  readonly provider = 'acpx'
  readonly modelId: string
  readonly supportedUrls: Record<string, RegExp[]> = {}

  private readonly providerInstance: AcpxProvider
  private readonly opts: AcpxLanguageModelOptions

  constructor(
    providerInstance: AcpxProvider,
    opts: AcpxLanguageModelOptions = {},
  ) {
    this.providerInstance = providerInstance
    this.opts = opts
    this.modelId = opts.agent ?? providerInstance.settings.agent
  }

  async doStream(
    callOptions: LanguageModelV2CallOptions,
  ): Promise<DoStreamResult> {
    const { handle, sessionKey } = await this.providerInstance.ensureHandle(
      this.opts,
    )
    const isFresh = this.providerInstance.markSessionKeyUsed(sessionKey)

    const { text, attachments } = convertPrompt({
      prompt: callOptions.prompt,
      responseFormat: callOptions.responseFormat,
      mode: isFresh ? 'fresh' : 'continuation',
    })

    const turn = this.providerInstance.runtime.startTurn({
      handle,
      text,
      attachments: attachments.length > 0 ? attachments : undefined,
      mode: this.opts.mode === 'steer' ? 'steer' : 'prompt',
      requestId: this.providerInstance.generateId(),
      timeoutMs:
        this.providerInstance.settings.turnTimeoutMs ?? DEFAULT_TURN_TIMEOUT_MS,
      signal: callOptions.abortSignal,
    })

    const translator = new EventTranslator({
      generateId: this.providerInstance.generateId,
    })

    let stream = createTranslatingStream(turn, translator)
    if (callOptions.responseFormat?.type === 'json') {
      stream = stream.pipeThrough(createJsonCleanupTransform())
    }

    return {
      stream,
      request: { body: { agent: this.modelId, sessionKey } },
      response: { headers: {} },
    }
  }

  async doGenerate(
    callOptions: LanguageModelV2CallOptions,
  ): Promise<DoGenerateResult> {
    const { stream, request, response } = await this.doStream(callOptions)
    return await accumulateStream(stream, request, response)
  }
}

function createTranslatingStream(
  turn: AcpRuntimeTurn,
  translator: EventTranslator,
): ReadableStream<LanguageModelV2StreamPart> {
  return new ReadableStream<LanguageModelV2StreamPart>({
    async start(controller) {
      try {
        for await (const event of turn.events) {
          for (const part of translator.translate(event))
            controller.enqueue(part)
        }
        const result = await turn.result
        for (const part of translator.flush()) controller.enqueue(part)
        for (const part of translator.errorPartIfFailed(result))
          controller.enqueue(part)
        controller.enqueue(translator.finish({ result }))
      } catch (err) {
        controller.enqueue({
          type: 'error',
          error: err instanceof Error ? err : new Error(String(err)),
        })
      } finally {
        controller.close()
      }
    },
  })
}

interface Accumulator {
  content: LanguageModelV2Content[]
  textBuffers: Map<string, string>
  reasoningBuffers: Map<string, string>
  finishReason: LanguageModelV2FinishReason
  usage: LanguageModelV2Usage
  providerMetadata?: SharedV2ProviderMetadata
}

function newAccumulator(): Accumulator {
  return {
    content: [],
    textBuffers: new Map(),
    reasoningBuffers: new Map(),
    finishReason: 'unknown',
    usage: EMPTY_USAGE,
  }
}

function appendBuffer(
  map: Map<string, string>,
  id: string,
  delta: string,
): void {
  map.set(id, (map.get(id) ?? '') + delta)
}

function flushBuffer(
  map: Map<string, string>,
  id: string,
  type: 'text' | 'reasoning',
  acc: Accumulator,
): void {
  const value = map.get(id) ?? ''
  if (value) acc.content.push({ type, text: value })
  map.delete(id)
}

function applyPart(part: LanguageModelV2StreamPart, acc: Accumulator): void {
  switch (part.type) {
    case 'text-start':
      acc.textBuffers.set(part.id, '')
      return
    case 'text-delta':
      appendBuffer(acc.textBuffers, part.id, part.delta)
      return
    case 'text-end':
      flushBuffer(acc.textBuffers, part.id, 'text', acc)
      return
    case 'reasoning-start':
      acc.reasoningBuffers.set(part.id, '')
      return
    case 'reasoning-delta':
      appendBuffer(acc.reasoningBuffers, part.id, part.delta)
      return
    case 'reasoning-end':
      flushBuffer(acc.reasoningBuffers, part.id, 'reasoning', acc)
      return
    case 'tool-call':
      acc.content.push({
        type: 'tool-call',
        toolCallId: part.toolCallId,
        toolName: part.toolName,
        input: part.input,
        providerExecuted: part.providerExecuted,
      })
      return
    case 'tool-result':
      acc.content.push({
        type: 'tool-result',
        toolCallId: part.toolCallId,
        toolName: part.toolName,
        result: part.result,
        ...(part.isError ? { isError: true } : {}),
      })
      return
    case 'finish':
      acc.finishReason = part.finishReason
      acc.usage = part.usage
      if (part.providerMetadata) acc.providerMetadata = part.providerMetadata
      return
    case 'error':
      throw part.error instanceof Error
        ? part.error
        : new Error(String(part.error))
  }
}

async function accumulateStream(
  stream: ReadableStream<LanguageModelV2StreamPart>,
  request: { body: unknown },
  response: { headers: Record<string, string> },
): Promise<DoGenerateResult> {
  const reader = stream.getReader()
  const acc = newAccumulator()
  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    applyPart(value, acc)
  }

  return {
    content: acc.content,
    finishReason: acc.finishReason,
    usage: acc.usage,
    providerMetadata: acc.providerMetadata,
    request,
    response,
    warnings: [],
  }
}
