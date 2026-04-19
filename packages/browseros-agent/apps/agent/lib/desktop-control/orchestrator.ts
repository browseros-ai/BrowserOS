/**
 * DesktopOrchestrator — vision-action loop for desktop automation.
 *
 * Implements the core loop:
 *   screenshot → send to LLM vision → parse action → execute → repeat
 *
 * Uses the agent server's chat endpoint for vision analysis so that
 * all provider resolution and streaming logic is reused.
 *
 * @module desktop-control/orchestrator
 */

import type {
  DesktopAction,
  DesktopControlService,
  OrchestratorConfig,
  OrchestratorResult,
  VisionAnalysisRequest,
  VisionAnalysisResponse,
} from './types'

/** System prompt for the vision model acting as a desktop agent. */
const VISION_AGENT_SYSTEM_PROMPT = `You are a desktop automation agent. You will receive a screenshot of the current desktop state and a task to complete.

Analyze the screenshot and determine the next action to take. Respond with a JSON object containing:
- "reasoning": brief explanation of what you see and what you plan to do
- "action": the next action to execute

Action types and their params:
- {"type":"mouse_move","params":{"x":100,"y":200}}
- {"type":"mouse_click","params":{"x":100,"y":200,"button":"left","clickType":"single"}}
- {"type":"mouse_double_click","params":{"x":100,"y":200}}
- {"type":"mouse_right_click","params":{"x":100,"y":200}}
- {"type":"mouse_drag","params":{"fromX":100,"fromY":100,"toX":300,"toY":300,"duration":300,"steps":20}}
- {"type":"mouse_scroll","params":{"x":100,"y":200,"amount":3,"direction":"down"}}
- {"type":"keyboard_type","params":{"text":"hello world","keyDelay":10}}
- {"type":"keyboard_key","params":{"key":"enter","modifiers":[]}}
- {"type":"keyboard_hotkey","params":{"key":"c","modifiers":["control"]}}
- {"type":"file_dialog","params":{"filePath":"/path/to/file","confirm":true,"timeout":5000}}
- {"type":"screenshot","params":{}}
- {"type":"wait","params":{"ms":1000}}
- {"type":"done","params":{}}

Respond ONLY with valid JSON. No markdown fences, no extra text.`

/**
 * DesktopOrchestrator drives the vision-action loop for desktop tasks.
 * @public
 */
export class DesktopOrchestrator {
  private service: DesktopControlService
  private agentServerUrl: string
  private providerId?: string

  constructor(deps: {
    service: DesktopControlService
    agentServerUrl: string
    providerId?: string
  }) {
    this.service = deps.service
    this.agentServerUrl = deps.agentServerUrl
    this.providerId = deps.providerId
  }

  /**
   * Run the vision-action loop until the task is done or maxIterations is reached.
   */
  async run(task: string, config?: OrchestratorConfig): Promise<OrchestratorResult> {
    const maxIterations = config?.maxIterations ?? 20
    const iterationDelay = config?.iterationDelay ?? 1000
    const actionDelay = config?.actionDelay ?? 500
    const signal = config?.signal ?? config?.signal

    const actions: DesktopAction[] = []
    let finalReasoning = ''
    let completed = false

    for (let i = 0; i < maxIterations; i++) {
      // Check for cancellation
      if (signal?.aborted) {
        finalReasoning = 'Cancelled by user'
        break
      }

      // Step 1: Capture screenshot
      const screenshot = await this.service.captureScreenshot()

      // Step 2: Send to vision model for analysis
      const analysisRequest: VisionAnalysisRequest = {
        screenshot: screenshot.base64,
        mimeType: screenshot.mimeType,
        task,
        previousActions: actions,
      }

      const analysis = await this.analyseScreenshot(analysisRequest)
      finalReasoning = analysis.reasoning

      // Step 3: Execute the parsed action
      const action = analysis.action
      actions.push(action)

      if (action.type === 'done') {
        completed = true
        break
      }

      await this.executeAction(action)

      // Step 4: Wait before next iteration
      await this.sleep(actionDelay)

      if (i < maxIterations - 1 && iterationDelay > 0) {
        await this.sleep(iterationDelay)
      }
    }

    return {
      completed,
      iterations: actions.length,
      actions,
      finalReasoning,
    }
  }

  // ─── Private Helpers ──────────────────────────────────────────────

  /**
   * Send a screenshot and task to the LLM for analysis.
   *
   * Uses the agent server's /chat endpoint with the screenshot
   * embedded as a base64 image in the message content.
   */
  private async analyseScreenshot(
    request: VisionAnalysisRequest,
  ): Promise<VisionAnalysisResponse> {
    try {
      // Build the user message with inline image
      const messageContent = [
        {
          type: 'text',
          text: `Task: ${request.task}\n\nPrevious actions: ${JSON.stringify(request.previousActions.map((a) => a.type))}\n\nAnalyze this screenshot and determine the next action. Respond with JSON only.`,
        },
        {
          type: 'image_url',
          image_url: {
            url: `data:${request.mimeType};base64,${request.screenshot}`,
          },
        },
      ]

      const response = await fetch(`${this.agentServerUrl}/chat`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          messages: [
            { role: 'system', content: VISION_AGENT_SYSTEM_PROMPT },
            { role: 'user', content: messageContent },
          ],
          mode: 'chat',
          providerId: this.providerId,
          supportsImages: true,
        }),
      })

      if (!response.ok) {
        throw new Error(
          `Vision analysis failed: ${response.status} ${response.statusText}`,
        )
      }

      const result = (await response.json()) as { text?: string }
      return this.parseVisionResponse(result.text ?? '')
    } catch (error) {
      // If analysis fails, return a "done" action to gracefully stop
      return {
        reasoning: `Vision analysis error: ${error instanceof Error ? error.message : String(error)}`,
        action: { type: 'done', params: {} },
      }
    }
  }

  /**
   * Parse the LLM's text response into a structured VisionAnalysisResponse.
   */
  private parseVisionResponse(text: string): VisionAnalysisResponse {
    try {
      // Strip markdown code fences if present
      const cleaned = text
        .replace(/^```(?:json)?\s*\n?/m, '')
        .replace(/\n?```\s*$/m, '')
        .trim()

      const parsed = JSON.parse(cleaned) as {
        reasoning?: string
        action?: DesktopAction
      }

      return {
        reasoning: parsed.reasoning ?? '',
        action: parsed.action ?? { type: 'done', params: {} },
      }
    } catch {
      // If JSON parsing fails, treat as done
      return {
        reasoning: `Failed to parse LLM response: ${text.slice(0, 200)}`,
        action: { type: 'done', params: {} },
      }
    }
  }

  /**
   * Execute a single desktop action using the DesktopControlService.
   */
  private async executeAction(action: DesktopAction): Promise<void> {
    const p = action.params

    switch (action.type) {
      case 'mouse_move':
        await this.service.mouseMove(
          { x: p.x as number, y: p.y as number },
        )
        break

      case 'mouse_click':
        await this.service.mouseClick(
          { x: p.x as number, y: p.y as number },
          {
            button: (p.button as 'left' | 'right' | 'middle') ?? 'left',
            clickType: (p.clickType as 'single' | 'double') ?? 'single',
          },
        )
        break

      case 'mouse_double_click':
        await this.service.mouseClick(
          { x: p.x as number, y: p.y as number },
          { clickType: 'double' },
        )
        break

      case 'mouse_right_click':
        await this.service.mouseClick(
          { x: p.x as number, y: p.y as number },
          { button: 'right' },
        )
        break

      case 'mouse_drag':
        await this.service.mouseDrag(
          { x: p.fromX as number, y: p.fromY as number },
          { x: p.toX as number, y: p.toY as number },
          {
            duration: (p.duration as number) ?? 300,
            steps: (p.steps as number) ?? 20,
          },
        )
        break

      case 'mouse_scroll':
        await this.service.mouseScroll(
          { x: p.x as number, y: p.y as number },
          {
            amount: (p.amount as number) ?? 3,
            direction: (p.direction as 'up' | 'down' | 'left' | 'right') ?? 'down',
          },
        )
        break

      case 'keyboard_type':
        await this.service.typeText(
          (p.text as string) ?? '',
          { keyDelay: (p.keyDelay as number) ?? 10 },
        )
        break

      case 'keyboard_key':
        await this.service.pressKey({
          key: (p.key as string) ?? 'enter',
          modifiers: (p.modifiers as Array<'alt' | 'control' | 'shift' | 'meta'>) ?? [],
        })
        break

      case 'keyboard_hotkey':
        await this.service.pressKey({
          key: (p.key as string) ?? '',
          modifiers: (p.modifiers as Array<'alt' | 'control' | 'shift' | 'meta'>) ?? [],
        })
        break

      case 'file_dialog':
        await this.service.handleFileDialog({
          filePath: (p.filePath as string) ?? '',
          confirm: (p.confirm as boolean) ?? true,
          timeout: (p.timeout as number) ?? 5000,
        })
        break

      case 'screenshot':
        // No-op: the next loop iteration will capture a screenshot anyway
        break

      case 'wait':
        await this.sleep((p.ms as number) ?? 1000)
        break

      case 'done':
        // Nothing to execute
        break
    }
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
