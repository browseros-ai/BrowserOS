import { randomUUID } from 'node:crypto'
import { MAX_ACTIONS_PER_DELEGATION } from '../../../../constants'
import { McpClient, type McpToolResult } from '../../../../utils/mcp-client'
import { sleep } from '../../../../utils/sleep'
import type {
  ExecutorConfig,
  ExecutorResult,
} from '../../../orchestrator-executor/types'
import type { ExecutorCallbacks } from '../../executor-backend'
import {
  extractCladoThinking,
  formatCladoHistory,
  getCladoActionSignature,
  parseCladoActions,
  summarizeCladoPrediction,
} from './clado-actions'
import {
  normalizeCladoDirection,
  normalizeCladoPressKey,
  normalizeCladoScrollAmount,
  prepareCladoToolCall,
  resolveCladoPoint,
} from './clado-browser-driver'
import { CladoActionClient } from './clado-client'
import {
  CLADO_ACTION_PROVIDER,
  type CladoAction,
  type CladoActionPoint,
  type CladoActionResponse,
  type CladoViewport,
  isCladoActionProvider,
} from './types'

const MAX_CONSECUTIVE_PARSE_FAILURES = 3

function asErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

export class CladoActionExecutor {
  private readonly mcpClient: McpClient
  private readonly cladoClient: CladoActionClient
  private readonly pageId: number
  private callbacks: ExecutorCallbacks = {}
  private stepsUsed = 0
  private viewport: CladoViewport | null = null
  private lastPoint: CladoActionPoint | null = null
  private currentUrl = ''

  constructor(
    config: ExecutorConfig,
    serverUrl: string,
    initialPageId?: number,
  ) {
    if (!isCladoActionProvider(config.provider)) {
      throw new Error(
        `CladoActionExecutor requires provider="${CLADO_ACTION_PROVIDER}"`,
      )
    }
    this.mcpClient = new McpClient(`${serverUrl}/mcp`)
    this.cladoClient = new CladoActionClient({
      baseUrl: config.baseUrl,
      apiKey: config.apiKey,
    })
    this.pageId = initialPageId ?? 1
  }

  setCallbacks(callbacks: ExecutorCallbacks): void {
    this.callbacks = callbacks
  }

  getTotalSteps(): number {
    return this.stepsUsed
  }

  async close(): Promise<void> {
    await this.mcpClient.close()
  }

  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: action-loop orchestration; refactor tracked separately
  async execute(
    instruction: string,
    signal?: AbortSignal,
  ): Promise<ExecutorResult> {
    this.viewport = null
    this.lastPoint = null

    const startSteps = this.stepsUsed
    const toolsUsed = new Set<string>()
    const actionHistory: CladoAction[] = []
    let predictionCalls = 0
    const thinkingTrace: string[] = []
    let consecutiveParseFailures = 0
    let finalAnswer: string | undefined

    let status: ExecutorResult['status'] = 'done'
    let reason = 'Goal executed.'

    for (let step = 0; step < MAX_ACTIONS_PER_DELEGATION; step++) {
      if (signal?.aborted) {
        status = 'timeout'
        reason = 'Delegation aborted by timeout or cancellation.'
        break
      }

      let screenshotBase64: string
      try {
        screenshotBase64 = await this.captureScreenshotBase64(signal)
      } catch (error) {
        status = signal?.aborted ? 'timeout' : 'blocked'
        reason = `Could not capture screenshot: ${asErrorMessage(error)}`
        break
      }

      const historyForPrediction = formatCladoHistory(actionHistory)
      const actionToolCallId = randomUUID()
      const predictionInput = {
        instruction,
        history: historyForPrediction,
      }

      this.callbacks.onToolCallStart?.({
        toolCallId: actionToolCallId,
        toolName: 'clado_action_predict',
        input: predictionInput,
      })

      let prediction: CladoActionResponse
      try {
        prediction = await this.requestActionPrediction(
          instruction,
          screenshotBase64,
          actionHistory,
          signal,
        )
        predictionCalls++
        const thinking = extractCladoThinking(prediction.raw_response)
        if (thinking) {
          const previous = thinkingTrace[thinkingTrace.length - 1]
          if (previous !== thinking) {
            thinkingTrace.push(thinking)
          }
        }
      } catch (error) {
        const message = asErrorMessage(error)
        await this.callbacks.onStepFinish?.({
          toolCalls: [
            {
              toolCallId: actionToolCallId,
              toolName: 'clado_action_predict',
              input: predictionInput,
            },
          ],
          toolResults: [
            {
              toolCallId: actionToolCallId,
              toolName: 'clado_action_predict',
              output: { error: message },
            },
          ],
        })
        status = signal?.aborted ? 'timeout' : 'blocked'
        reason = `Clado action request failed: ${message}`
        break
      }

      const predictedActions = parseCladoActions(prediction)
      if (predictedActions.length === 0) {
        // Per Clado contract: HTTP 200 with action=null on parse failure.
        // Count as an invalid step so the model can self-correct on the
        // next call instead of dropping the trajectory.
        consecutiveParseFailures++
        const parseError =
          prediction.parse_error ?? 'no parsable <answer> in raw_response'
        actionHistory.push({
          action: 'invalid',
          text: `parse_error: ${parseError}`,
        })
        this.stepsUsed++
        await this.callbacks.onStepFinish?.({
          toolCalls: [
            {
              toolCallId: actionToolCallId,
              toolName: 'clado_action_predict',
              input: predictionInput,
            },
          ],
          toolResults: [
            {
              toolCallId: actionToolCallId,
              toolName: 'clado_action_predict',
              output: {
                prediction: summarizeCladoPrediction(prediction),
                parsedActions: [],
                parseError,
                consecutiveParseFailures,
              },
            },
          ],
        })

        if (consecutiveParseFailures >= MAX_CONSECUTIVE_PARSE_FAILURES) {
          status = 'blocked'
          reason = `Clado returned ${consecutiveParseFailures} consecutive unparseable responses.`
          break
        }
        continue
      }
      consecutiveParseFailures = 0

      let requestedStop = false
      const executionNotes: string[] = []
      for (const predictedAction of predictedActions) {
        try {
          reason = await this.executeAction(predictedAction, signal)
          executionNotes.push(reason)
          this.stepsUsed++
          await this.callbacks.onToolCallFinish?.()
        } catch (error) {
          const message = asErrorMessage(error)
          executionNotes.push(`Failed ${predictedAction.action}: ${message}`)
          await this.callbacks.onStepFinish?.({
            toolCalls: [
              {
                toolCallId: actionToolCallId,
                toolName: 'clado_action_predict',
                input: predictionInput,
              },
            ],
            toolResults: [
              {
                toolCallId: actionToolCallId,
                toolName: 'clado_action_predict',
                output: {
                  prediction: summarizeCladoPrediction(prediction),
                  parsedActions: predictedActions,
                  executed: executionNotes,
                },
              },
            ],
          })
          status = signal?.aborted ? 'timeout' : 'blocked'
          reason = `Action execution failed: ${message}`
          requestedStop = true
          break
        }

        actionHistory.push(predictedAction)
        if (predictedAction.action === 'end') {
          if (predictedAction.final_answer) {
            finalAnswer = predictedAction.final_answer
            reason = `Model requested end() with final_answer: ${predictedAction.final_answer.slice(0, 240)}`
          } else {
            reason = 'Model requested end() and marked task complete.'
          }
          requestedStop = true
          break
        }
      }

      if (status === 'done') {
        toolsUsed.add('clado_action_predict')
        await this.callbacks.onStepFinish?.({
          toolCalls: [
            {
              toolCallId: actionToolCallId,
              toolName: 'clado_action_predict',
              input: predictionInput,
            },
          ],
          toolResults: [
            {
              toolCallId: actionToolCallId,
              toolName: 'clado_action_predict',
              output: {
                prediction: summarizeCladoPrediction(prediction),
                parsedActions: predictedActions,
                executed: executionNotes,
              },
            },
          ],
        })
      }

      if (requestedStop) break
    }

    if (
      status === 'done' &&
      predictionCalls >= MAX_ACTIONS_PER_DELEGATION &&
      !signal?.aborted
    ) {
      status = 'blocked'
      reason = `Reached max action budget (${MAX_ACTIONS_PER_DELEGATION}) without a clear completion signal.`
    }

    if (signal?.aborted && status === 'done') {
      status = 'timeout'
      reason = 'Delegation aborted by timeout or cancellation.'
    }

    this.currentUrl = await this.getCurrentUrl(signal)

    const observation = this.buildObservation({
      status,
      reason,
      actions: actionHistory,
      url: this.currentUrl,
      thinkingTrace,
      finalAnswer,
    })

    return {
      observation,
      status,
      url: this.currentUrl,
      actionsPerformed: this.stepsUsed - startSteps,
      toolsUsed: [...toolsUsed],
    }
  }

  private async requestActionPrediction(
    instruction: string,
    imageBase64: string,
    actionHistory: CladoAction[],
    signal?: AbortSignal,
  ): Promise<CladoActionResponse> {
    return this.cladoClient.requestActionPrediction({
      instruction,
      imageBase64,
      actionHistory,
      signal,
    })
  }

  private async executeAction(
    action: CladoAction,
    signal?: AbortSignal,
  ): Promise<string> {
    switch (action.action) {
      case 'click':
      case 'double_click': {
        const point = await this.resolvePoint(action.x, action.y, signal)
        await this.runTool(
          'click_at',
          {
            x: point.x,
            y: point.y,
            clickCount: action.action === 'double_click' ? 2 : 1,
          },
          signal,
        )
        this.lastPoint = point
        return `Executed ${action.action} at (${point.x}, ${point.y}).`
      }

      case 'right_click': {
        const point = await this.resolvePoint(action.x, action.y, signal)
        await this.runTool(
          'click_at',
          {
            x: point.x,
            y: point.y,
            button: 'right',
            clickCount: 1,
          },
          signal,
        )
        this.lastPoint = point
        return `Executed right_click at (${point.x}, ${point.y}).`
      }

      case 'hover': {
        const point = await this.resolvePoint(action.x, action.y, signal)
        await this.runTool('hover_at', { x: point.x, y: point.y }, signal)
        this.lastPoint = point
        return `Hovered at (${point.x}, ${point.y}).`
      }

      case 'type': {
        const text = action.text ?? ''
        if (!text) throw new Error('type action missing text field')

        if (typeof action.x === 'number' && typeof action.y === 'number') {
          this.lastPoint = await this.resolvePoint(action.x, action.y, signal)
        }

        if (this.lastPoint) {
          await this.runTool(
            'type_at',
            { x: this.lastPoint.x, y: this.lastPoint.y, text, clear: false },
            signal,
          )
        } else {
          throw new Error(
            'type action: no coordinates available — cannot determine where to type. ' +
              'Provide x/y or hover/click the target field first.',
          )
        }
        return `Typed text (${Math.min(text.length, 120)} chars).`
      }

      case 'press_key': {
        const key = normalizeCladoPressKey(action.key)
        await this.runTool('press_key', { key }, signal)
        return `Pressed key "${key}".`
      }

      case 'scroll': {
        const direction = normalizeCladoDirection(action.direction)
        const amountPx = normalizeCladoScrollAmount(action.amount)
        const ticks = Math.max(1, Math.round(amountPx / 120))

        await this.runTool('scroll', { direction, amount: ticks }, signal)
        return `Scrolled ${direction} by ${ticks} ticks.`
      }

      case 'drag': {
        if (
          typeof action.startX !== 'number' ||
          typeof action.startY !== 'number' ||
          typeof action.endX !== 'number' ||
          typeof action.endY !== 'number'
        ) {
          throw new Error('drag action missing start/end coordinates')
        }
        const start = await this.resolvePoint(
          action.startX,
          action.startY,
          signal,
        )
        const end = await this.resolvePoint(action.endX, action.endY, signal)

        await this.runTool(
          'drag_at',
          { startX: start.x, startY: start.y, endX: end.x, endY: end.y },
          signal,
        )
        this.lastPoint = end
        return `Dragged from (${start.x}, ${start.y}) to (${end.x}, ${end.y}).`
      }

      case 'wait': {
        const waitSeconds = Math.max(
          1,
          Math.min(10, Math.round(action.time ?? 1)),
        )
        await sleep(waitSeconds * 1000, signal)
        return `Waited ${waitSeconds}s.`
      }

      case 'end': {
        return action.final_answer
          ? `Model requested end() with final_answer: ${action.final_answer.slice(0, 240)}`
          : 'Model requested end().'
      }

      default: {
        throw new Error(`Unsupported Clado action: ${action.action}`)
      }
    }
  }

  private async captureScreenshotBase64(signal?: AbortSignal): Promise<string> {
    // Clado contract is PNG or JPEG; use PNG for lossless input.
    const result = await this.runTool(
      'take_screenshot',
      { format: 'png' },
      signal,
    )

    const image = result.raw.content.find(
      (item) => item.type === 'image' && typeof item.data === 'string',
    )
    if (!image?.data) {
      throw new Error('Screenshot response did not include base64 image data')
    }

    return image.data
  }

  private async getViewport(signal?: AbortSignal): Promise<CladoViewport> {
    if (this.viewport) return this.viewport

    try {
      const result = await this.runTool(
        'evaluate_script',
        { function: '() => [window.innerWidth, window.innerHeight]' },
        signal,
      )
      const text = result.text
      const match = text.match(/\[\s*(\d+)\s*,\s*(\d+)\s*\]/s)
      if (match) {
        const width = Number.parseInt(match[1], 10)
        const height = Number.parseInt(match[2], 10)
        if (width > 0 && height > 0) {
          this.viewport = { width, height }
          return this.viewport
        }
      }
    } catch {
      // fallback below
    }

    this.viewport = { width: 1440, height: 900 }
    return this.viewport
  }

  private async resolvePoint(
    normalizedX: number | undefined,
    normalizedY: number | undefined,
    signal?: AbortSignal,
  ): Promise<CladoActionPoint> {
    const viewport = await this.getViewport(signal)
    return resolveCladoPoint(viewport, normalizedX, normalizedY)
  }

  private async getCurrentUrl(signal?: AbortSignal): Promise<string> {
    try {
      const result = await this.runTool(
        'evaluate_script',
        { function: '() => window.location.href' },
        signal,
      )
      const text = result.text
      const urlMatch = text.match(/https?:\/\/[^\s"`]+/i)
      return urlMatch ? urlMatch[0] : this.currentUrl
    } catch {
      return this.currentUrl
    }
  }

  private async runTool(
    toolName: string,
    args: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<{ raw: McpToolResult; text: string }> {
    if (signal?.aborted) {
      throw new Error('aborted')
    }

    const prepared = prepareCladoToolCall(toolName, args, this.pageId)

    try {
      const raw = await this.mcpClient.callTool(
        prepared.toolName,
        prepared.args,
      )
      const text = raw.content
        .map((item) => item.text)
        .filter((value): value is string => typeof value === 'string')
        .join('\n')

      if (raw.isError) {
        throw new Error(text || `${toolName} failed`)
      }

      return { raw, text }
    } catch (error) {
      throw new Error(`${toolName} failed: ${asErrorMessage(error)}`)
    }
  }

  private buildObservation(params: {
    status: ExecutorResult['status']
    reason: string
    actions: CladoAction[]
    url: string
    thinkingTrace: string[]
    finalAnswer?: string
  }): string {
    const { status, reason, actions, url, thinkingTrace, finalAnswer } = params
    const actionSummary =
      actions.length === 0
        ? 'No actions were executed.'
        : actions
            .slice(-5)
            .map(
              (action, idx) => `${idx + 1}. ${getCladoActionSignature(action)}`,
            )
            .join('\n')
    const thinkingSummary =
      thinkingTrace.length === 0
        ? ''
        : thinkingTrace
            .map((thinking, idx) => `Step ${idx + 1}: ${thinking}`)
            .join('\n\n')

    return [
      `Status: ${status}`,
      `Reason: ${reason}`,
      `URL: ${url || 'unknown'}`,
      finalAnswer ? `Final answer: ${finalAnswer}` : '',
      '',
      'Recent actions:',
      actionSummary,
      '',
      `Total model actions: ${actions.length}`,
      '',
      thinkingSummary ? `Model thinking trace:\n${thinkingSummary}` : '',
    ]
      .filter(Boolean)
      .join('\n')
  }
}
