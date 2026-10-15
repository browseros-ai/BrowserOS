import type { BrowserTab, UiMessageChunk } from '@browseros/server-api'
import { readUiMessageStream } from '@browseros/server-api-client'
import { chatErrorMessage } from '@browseros/shared/schemas/chat-error'
import { getAgentServerUrl } from '@/lib/browseros/helpers'
import { customServerEndpoints } from '@/lib/mcp/custom-server-endpoints'
import { mcpServerStorage } from '@/lib/mcp/mcpServerStorage'
import { buildChatRequestBody } from '@/lib/messaging/server/buildChatRequestBody'
import { serverApi, serverRequest } from '@/lib/server-api/client'
import type { ChatMode } from '@/modules/chat/chat-types'
import { personalizationStorage } from '../personalization/personalizationStorage'
import { scheduleSystemPrompt } from './scheduleSystemPrompt'
import type { ToolCallExecution } from './scheduleTypes'

export type ActiveTab = BrowserTab

export interface ChatServerRequest {
  message: string
  mode?: ChatMode
  conversationId?: string
  windowId?: number
  activeTab?: ActiveTab
  signal?: AbortSignal
  providerId?: string
}

export interface ChatServerResponse {
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

interface StreamParseState {
  fullText: string
  currentStepText: string
  lastTextBeforeToolCall: string
  executionSteps: string[]
  toolCallsMap: Map<string, ToolCallExecution>
  error: string | null
  receivedFinish: boolean
}

export async function getChatServerResponse(
  request: ChatServerRequest,
): Promise<ChatServerResponse> {
  const agentServerUrl = await getAgentServerUrl()
  // No provider lookup here any more. The server holds the list and the
  // selection, so a job names an id or names nothing and the server resolves
  // it. That also removes the guard this path needed when it did the lookup
  // itself: an unreachable list could not be told apart from an empty one, so
  // a job risked running on the built-in provider with the wrong credentials.
  const conversationId = request.conversationId ?? crypto.randomUUID()
  const personalization = await personalizationStorage.getValue()

  const mcpServers = (await mcpServerStorage.getValue()) ?? []
  const enabledMcpServers = mcpServers
    .filter((s) => s.type === 'managed')
    .map((s) => s.managedServerName)
    .filter((name): name is string => !!name)
  const customMcpServers = customServerEndpoints(mcpServers)

  const response = await serverRequest(
    serverApi(agentServerUrl).createChat(
      {
        messages: [{ role: 'user', content: request.message }],
        ...buildChatRequestBody({
          providerId: request.providerId,
          message: request.message,
          conversationId,
          mode: request.mode ?? 'agent',
          browserContext:
            request.activeTab ||
            request.windowId ||
            enabledMcpServers.length ||
            customMcpServers.length
              ? {
                  windowId: request.windowId,
                  activeTab: request.activeTab,
                  enabledMcpServers:
                    enabledMcpServers.length > 0
                      ? enabledMcpServers
                      : undefined,
                  customMcpServers:
                    customMcpServers.length > 0 ? customMcpServers : undefined,
                }
              : undefined,
          userSystemPrompt: `${personalization}\n${scheduleSystemPrompt}`,
          isScheduledTask: true,
        }),
      },
      { signal: request.signal },
    ),
    async (error, failedResponse) => {
      const body = await failedResponse
        ?.clone()
        .text()
        .catch(() => '')
      return (
        (body ? chatErrorMessage(body) : '') ||
        `Chat request failed: ${error.status} ${failedResponse?.statusText ?? ''}`
      )
    },
  )

  const parsed = await parseUIMessageStream(response, request.signal)

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

function processEvent(event: UiMessageChunk, state: StreamParseState): void {
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
      // Run history renders these strings verbatim, so unwrap the envelope the
      // server serializes into errorText.
      existingCall.error = chatErrorMessage(event.errorText)
    }
  } else if (event.type === 'error') {
    state.error = chatErrorMessage(event.errorText)
  } else if (event.type === 'finish') {
    state.receivedFinish = true
  }
}

async function parseUIMessageStream(
  response: Response,
  signal?: AbortSignal,
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

  try {
    for await (const event of readUiMessageStream(response, { signal })) {
      processEvent(event, state)
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
          ? error.message === 'The chat stream ended before [DONE]'
            ? (state.error ??
              'Stream ended unexpectedly without completion. The task may have been interrupted.')
            : error.message
          : String(error || 'Unknown error'),
    }
  }
}
