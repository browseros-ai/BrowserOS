import { createParser, type EventSourceMessage } from 'eventsource-parser'
import type { ChatMode } from '@/entrypoints/sidepanel/index/chatTypes'
import { getAgentServerUrl } from '@/lib/browseros/helpers'
import {
  createDefaultBrowserOSProvider,
  defaultProviderIdStorage,
  providersStorage,
} from '@/lib/llm-providers/storage'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { mcpServerStorage } from '@/lib/mcp/mcpServerStorage'
import { personalizationStorage } from '../personalization/personalizationStorage'
import { scheduleSystemPrompt } from './scheduleSystemPrompt'
import type { ToolCallExecution } from './scheduleTypes'

interface ActiveTab {
  id?: number
  url?: string
  title?: string
}

interface ChatServerRequest {
  message: string
  mode?: ChatMode
  conversationId?: string
  windowId?: number
  activeTab?: ActiveTab
  signal?: AbortSignal
  providerId?: string
}

interface ChatServerResponse {
  text: string
  conversationId: string
  finalResult: string
  executionLog: string
  toolCalls: ToolCallExecution[]
}

interface ParsedStreamResult {
  fullText: string
  finalResult: string
  executionLog: string
  toolCalls: ToolCallExecution[]
  error: string | null
}

type UIMessageEvent =
  | { type: 'text-delta'; id: string; delta: string }
  | {
      type: 'tool-input-available'
      toolCallId: string
      toolName: string
      input: unknown
    }
  | { type: 'tool-output-available'; toolCallId: string; output: unknown }
  | { type: 'tool-output-error'; toolCallId: string; errorText: string }
  | { type: 'error'; errorText: string }
  | { type: 'finish'; finishReason: string }

interface StreamParseState {
  fullText: string
  currentStepText: string
  lastTextBeforeToolCall: string
  executionSteps: string[]
  toolCallsMap: Map<string, ToolCallExecution>
  error: string | null
  receivedFinish: boolean
}

const getDefaultProvider = async (): Promise<LlmProviderConfig | null> => {
  const providers = await providersStorage.getValue()
  if (!providers?.length) return null

  const defaultProviderId = await defaultProviderIdStorage.getValue()
  const defaultProvider = providers.find((p) => p.id === defaultProviderId)
  return defaultProvider ?? providers[0] ?? null
}

// Resolve provider by ID, falling back to global default
const resolveProvider = async (
  providerId?: string,
): Promise<LlmProviderConfig> => {
  if (providerId) {
    const providers = await providersStorage.getValue()
    const match = providers?.find((p) => p.id === providerId)
    if (match) return match
  }
  return (await getDefaultProvider()) ?? createDefaultBrowserOSProvider()
}

export async function getChatServerResponse(
  request: ChatServerRequest,
): Promise<ChatServerResponse> {
  const agentServerUrl = await getAgentServerUrl()
  const provider = await resolveProvider(request.providerId)
  const conversationId = request.conversationId ?? crypto.randomUUID()
  const personalization = await personalizationStorage.getValue()

  const mcpServers = (await mcpServerStorage.getValue()) ?? []
  const enabledMcpServers = mcpServers
    .filter((s) => s.type === 'managed')
    .map((s) => s.managedServerName)
    .filter((name): name is string => !!name)
  const customMcpServers = mcpServers
    .filter((s) => s.type === 'custom' && !!s.config?.url)
    // biome-ignore lint/style/noNonNullAssertion: filter guarantees url exists
    .map((s) => ({ name: s.displayName, url: s.config!.url }))

  const response = await fetch(`${agentServerUrl}/chat`, {
    method: 'POST',
    signal: request.signal,
    headers: {
      'Content-Type': 'application/json',
    },
    // Important: this chat logic is also used in apps/agent/entrypoints/sidepanel/index/useChatSession.ts for sidepanel conversation. Make sure to keep them in sync for any future changes.
    body: JSON.stringify({
      messages: [{ role: 'user', content: request.message }],
      message: request.message,
      provider: provider?.type,
      providerType: provider?.type,
      providerName: provider?.name,
      apiKey: provider?.apiKey,
      baseUrl: provider?.baseUrl,
      conversationId,
      model: provider?.modelId ?? 'default',
      mode: request.mode ?? 'agent',
      contextWindowSize: provider?.contextWindow,
      temperature: provider?.temperature,
      resourceName: provider?.resourceName,
      accessKeyId: provider?.accessKeyId,
      secretAccessKey: provider?.secretAccessKey,
      region: provider?.region,
      sessionToken: provider?.sessionToken,
      browserContext:
        request.activeTab ||
        request.windowId ||
        enabledMcpServers.length ||
        customMcpServers.length
          ? {
              windowId: request.windowId,
              activeTab: request.activeTab,
              enabledMcpServers:
                enabledMcpServers.length > 0 ? enabledMcpServers : undefined,
              customMcpServers:
                customMcpServers.length > 0 ? customMcpServers : undefined,
            }
          : undefined,
      userSystemPrompt: `${personalization}\n${scheduleSystemPrompt}`,
      isScheduledTask: true,
      supportsImages: provider?.supportsImages,
    }),
  })

  if (!response.ok) {
    throw new Error(
      `Chat request failed: ${response.status} ${response.statusText}`,
    )
  }

  const parsed = await parseUIMessageStream(response)

  if (parsed.error) {
    throw new Error(parsed.error)
  }

  return {
    text: parsed.fullText,
    conversationId,
    finalResult: parsed.finalResult,
    executionLog: parsed.executionLog,
    toolCalls: parsed.toolCalls,
  }
}

function processEvent(event: UIMessageEvent, state: StreamParseState): void {
  if (event.type === 'text-delta') {
    const text = event.delta
    state.fullText += text
    state.currentStepText += text
    state.lastTextBeforeToolCall += text
  } else if (event.type === 'tool-input-available') {
    const toolCall: ToolCallExecution = {
      id: event.toolCallId,
      name: event.toolName,
      input: event.input,
      timestamp: new Date().toISOString(),
    }

    state.toolCallsMap.set(event.toolCallId, toolCall)

    if (state.currentStepText.trim()) {
      state.executionSteps.push(state.currentStepText.trim())
      state.currentStepText = ''
    }
  } else if (event.type === 'tool-output-available') {
    const existingCall = state.toolCallsMap.get(event.toolCallId)
    if (existingCall) {
      existingCall.output = event.output
    }
  } else if (event.type === 'tool-output-error') {
    const existingCall = state.toolCallsMap.get(event.toolCallId)
    if (existingCall) {
      existingCall.error = event.errorText
    }
  } else if (event.type === 'error') {
    state.error = event.errorText
  } else if (event.type === 'finish') {
    state.receivedFinish = true
  }
}

async function parseUIMessageStream(
  response: Response,
): Promise<ParsedStreamResult> {
  if (!response.body) {
    throw new Error('Response body is not readable')
  }

  const state: StreamParseState = {
    fullText: '',
    currentStepText: '',
    lastTextBeforeToolCall: '',
    executionSteps: [],
    toolCallsMap: new Map(),
    error: null,
    receivedFinish: false,
  }

  const parser = createParser({
    onEvent(event: EventSourceMessage) {
      if (event.data === '[DONE]') return

      try {
        const parsedEvent = JSON.parse(event.data) as UIMessageEvent
        processEvent(parsedEvent, state)
      } catch {
        // Ignore invalid JSON events
      }
    },
  })

  try {
    const reader = response.body.getReader()
    const decoder = new TextDecoder()

    while (true) {
      const { done, value } = await reader.read()

      if (done) break

      const chunk = decoder.decode(value, { stream: true })
      parser.feed(chunk)
    }

    if (!state.receivedFinish && !state.error) {
      state.error =
        'Stream ended unexpectedly without completion. The task may have been interrupted.'
    }

    const finalResult = state.currentStepText.trim()
      ? state.currentStepText.trim()
      : state.lastTextBeforeToolCall.trim()

    const allSteps = [...state.executionSteps]
    if (finalResult) {
      allSteps.push(finalResult)
    }

    return {
      fullText: state.fullText,
      finalResult,
      executionLog: allSteps.join('\n\n'),
      toolCalls: Array.from(state.toolCallsMap.values()),
      error: state.error,
    }
  } catch (error) {
    return {
      fullText: state.fullText,
      finalResult: '',
      executionLog: state.executionSteps.join('\n\n'),
      toolCalls: Array.from(state.toolCallsMap.values()),
      error:
        error instanceof Error
          ? error.message
          : String(error || 'Unknown error'),
    }
  }
}
