/**
 * OrchestratorAgent - ToolLoopAgent with a single delegate tool
 *
 * The orchestrator delegates goals to an executor and produces a final text answer.
 * Uses AI SDK ToolLoopAgent — the SDK handles the turn loop automatically.
 */

import { createLanguageModel } from '@browseros/server/agent/tool-loop/provider-factory'
import type { ResolvedAgentConfig } from '@browseros/server/agent/types'
import { stepCountIs, ToolLoopAgent, tool } from 'ai'
import { z } from 'zod'
import type { ExecutorFactory, ExecutorResult } from './types'
import { LIMITS, ORCHESTRATOR_DEFAULTS } from './types'

const ORCHESTRATOR_SYSTEM_PROMPT = `You are a task orchestrator for browser automation. You break a user's task into goal-level steps, delegate each to an executor, and report the final result.

## Your Tool
- delegate(instruction): Send a goal-level instruction to a browser executor

## How to Finish
When the task is complete, respond with a plain text message summarizing the result. Do NOT call delegate — just write your final answer as text. The system will capture your text as the answer.

If the task cannot be completed, respond with text explaining what went wrong and why.

## Rules

1. You CANNOT see the browser. The executor can. You plan WHAT, the executor handles HOW.

2. One goal per delegation. Be specific and goal-oriented:
   - Good: "Navigate to news.ycombinator.com/best"
   - Good: "Click the comments link of the 2nd post on the page"
   - Bad: "Go to HN and find posts and click things"

3. After each delegation, read the executor's observation and decide:
   - Task accomplished? → Respond with your final answer text (no tool call)
   - Need more steps? → Call delegate() with the next instruction
   - Stuck? → Try a different approach or respond with failure text

4. Every delegation uses a fresh executor with clean context. Write each instruction so it can be executed independently.

## Reading Executor Results

Each executor result includes:
- Status: done (goal achieved), blocked (stuck), timeout (ran out of time)
- Observation: what the executor saw and did
- URL: current page URL
- Actions performed: number of browser actions taken

Use the observation to understand the current browser state and plan your next step.`

export interface OrchestratorAgentOptions {
  executorFactory: ExecutorFactory
}

export interface OrchestratorAgentResult {
  success: boolean
  answer: string | null
  reason: string | null
  delegationCount: number
  totalExecutorSteps: number
  turns: number
}

interface AgentRunner {
  generate(params: { prompt: string; abortSignal?: AbortSignal }): Promise<{
    text: string
    toolCalls?: { toolCallId: string; toolName: string }[]
  }>
}

export class OrchestratorAgent {
  private constructor(
    private agent: AgentRunner,
    private state: {
      delegationCount: number
      totalExecutorSteps: number
      lastObservation: string
    },
    private maxTurns: number,
  ) {}

  static async create(
    resolvedConfig: ResolvedAgentConfig & { maxTurns?: number },
    options: OrchestratorAgentOptions,
  ): Promise<OrchestratorAgent> {
    const { model } = await createLanguageModel(resolvedConfig)
    const state = {
      delegationCount: 0,
      totalExecutorSteps: 0,
      lastObservation: '',
    }
    const maxTurns = resolvedConfig.maxTurns ?? ORCHESTRATOR_DEFAULTS.maxTurns

    const delegate = tool({
      description:
        'Delegate a goal-level instruction to a browser executor. The executor will perform browser actions to achieve the goal and report back an observation.',
      inputSchema: z.object({
        instruction: z
          .string()
          .describe(
            'A clear, goal-level instruction for the executor. One goal per delegation.',
          ),
      }),
      execute: async ({ instruction }, { abortSignal }) => {
        if (state.totalExecutorSteps >= LIMITS.maxTotalSteps) {
          return `Step budget exhausted (${LIMITS.maxTotalSteps} steps used). Cannot delegate further.`
        }
        state.delegationCount++

        const delegationController = new AbortController()
        const timeoutId = setTimeout(
          () => delegationController.abort(),
          LIMITS.delegationTimeoutMs,
        )

        const onParentAbort = () => delegationController.abort()
        abortSignal?.addEventListener('abort', onParentAbort, { once: true })

        let result: ExecutorResult
        try {
          result = await options.executorFactory(
            instruction,
            delegationController.signal,
          )
        } catch (err) {
          const msg = err instanceof Error ? err.message : String(err)
          result = {
            observation: `Delegation failed: ${msg}`,
            status: 'timeout',
            url: '',
            actionsPerformed: 0,
            toolsUsed: [],
          }
        } finally {
          clearTimeout(timeoutId)
          abortSignal?.removeEventListener('abort', onParentAbort)
        }

        state.totalExecutorSteps += result.actionsPerformed

        const statusNote = result.status === 'timeout' ? ' (TIMED OUT)' : ''
        const observation = `Executor Result:
- Status: ${result.status}${statusNote}
- Actions: ${result.actionsPerformed}
- URL: ${result.url || 'unknown'}

Observation:
${result.observation}`
        state.lastObservation = observation
        return observation
      },
    })

    const agent = new ToolLoopAgent({
      model,
      instructions: ORCHESTRATOR_SYSTEM_PROMPT,
      tools: { delegate },
      stopWhen: [stepCountIs(maxTurns)],
    })

    return new OrchestratorAgent(agent, state, maxTurns)
  }

  async run(
    taskQuery: string,
    signal?: AbortSignal,
  ): Promise<OrchestratorAgentResult> {
    let answer: string | null = null
    let success = false
    let reason: string | null = null

    try {
      const result = await this.agent.generate({
        prompt: taskQuery,
        abortSignal: signal,
      })

      answer = result.text || null
      const usedFallback = !answer && !!this.state.lastObservation
      if (usedFallback) {
        answer = this.state.lastObservation
      }
      success = answer !== null && !usedFallback
    } catch (err) {
      if (signal?.aborted) {
        reason = 'Aborted by eval timeout'
      } else {
        reason = err instanceof Error ? err.message : String(err)
      }
    }

    if (!success && !reason) {
      if (this.state.totalExecutorSteps >= LIMITS.maxTotalSteps) {
        reason = `Exceeded maximum total steps (${LIMITS.maxTotalSteps})`
      } else {
        reason = `Exceeded maximum orchestrator turns (${this.maxTurns})`
      }
    }

    return {
      success,
      answer,
      reason,
      delegationCount: this.state.delegationCount,
      totalExecutorSteps: this.state.totalExecutorSteps,
      turns: this.state.delegationCount,
    }
  }
}
