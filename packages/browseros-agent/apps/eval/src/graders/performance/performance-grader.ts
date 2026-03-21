import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { query } from '@anthropic-ai/claude-agent-sdk'
import type { GraderResult } from '../../types'
import type { Grader, GraderInput } from '../types'
import {
  buildUserPrompt,
  DEFAULT_AXES,
  PERFORMANCE_SYSTEM_PROMPT,
} from './axes'
import { extractMetrics } from './metadata-extractor'
import {
  type AxisDefinition,
  PERFORMANCE_EVAL_SCHEMA,
  type PerformanceEvalResponse,
  type PerformanceGraderOptions,
} from './types'

export const DEFAULT_MAX_TURNS = 100
export const DEFAULT_MAX_BUDGET_USD = 100
export const DEFAULT_PASS_THRESHOLD = 75
const DEFAULT_MODEL = 'claude-opus-4-5-20251101'
const GRADER_TIMEOUT_MS = 300_000

export class PerformanceGrader implements Grader {
  name = 'performance_grader'
  private model: string
  private axes: AxisDefinition[]
  private passThreshold: number
  private maxTurns: number
  private maxBudgetUsd: number

  constructor(
    _apiKey?: string,
    _baseUrl?: string,
    model?: string,
    options?: PerformanceGraderOptions,
  ) {
    this.model = model || DEFAULT_MODEL
    this.axes = options?.axes || DEFAULT_AXES
    this.passThreshold = options?.passThreshold ?? DEFAULT_PASS_THRESHOLD
    this.maxTurns = options?.maxTurns ?? DEFAULT_MAX_TURNS
    this.maxBudgetUsd = options?.maxBudgetUsd ?? DEFAULT_MAX_BUDGET_USD
  }

  async grade(input: GraderInput): Promise<GraderResult> {
    try {
      // Read termination reason from metadata.json
      let terminationReason = 'unknown'
      try {
        const metadataRaw = await readFile(
          join(input.outputDir, 'metadata.json'),
          'utf-8',
        )
        const metadata = JSON.parse(metadataRaw)
        terminationReason = metadata.termination_reason || 'unknown'
      } catch {
        // metadata.json may not exist
      }

      const metrics = extractMetrics(
        input.messages,
        input.screenshotCount,
        terminationReason,
      )

      const systemPrompt = PERFORMANCE_SYSTEM_PROMPT.replace(
        /\{screenshot_count\}/g,
        String(metrics.screenshotCount),
      )

      const userPrompt = buildUserPrompt(
        input.task.query,
        input.finalAnswer,
        metrics,
        this.axes,
        input.expectedAnswer,
      )

      const response = await this.runAgent(
        systemPrompt,
        userPrompt,
        input.outputDir,
      )

      if (!response) {
        return {
          score: 0,
          pass: false,
          reasoning: 'Agent returned no result',
          details: { error: true, grader: this.name },
        }
      }

      if (response.subtype !== 'success') {
        return {
          score: 0,
          pass: false,
          reasoning: `Agent failed: ${response.subtype}`,
          details: {
            error: true,
            grader: this.name,
            costUsd: response.total_cost_usd,
          },
        }
      }

      const parsed = this.parseResponse(response.structured_output)
      if (!parsed) {
        return {
          score: 0,
          pass: false,
          reasoning: `Failed to parse agent output: ${response.result}`,
          details: { error: true, grader: this.name },
        }
      }

      const axisResults: Record<
        string,
        { score: number; weight: number; reasoning: string }
      > = {}
      let compositeScore = 0

      for (const axisScore of parsed.axes) {
        const axisDef = this.axes.find((a) => a.name === axisScore.axis)
        const weight = axisDef?.weight ?? 0
        axisResults[axisScore.axis] = {
          score: axisScore.score,
          weight,
          reasoning: axisScore.reasoning,
        }
        compositeScore += axisScore.score * weight
      }

      const expectedAxes = new Set(this.axes.map((a) => a.name))
      const returnedAxes = new Set(parsed.axes.map((a) => a.axis))
      const missingAxes = [...expectedAxes].filter((n) => !returnedAxes.has(n))
      if (missingAxes.length > 0) {
        console.warn(
          `Perf grader: LLM returned ${returnedAxes.size}/${expectedAxes.size} axes, missing: ${missingAxes.join(', ')}`,
        )
      }

      return {
        score: compositeScore / 100,
        pass: compositeScore >= this.passThreshold,
        reasoning: this.buildReasoningSummary(
          parsed,
          compositeScore,
          missingAxes,
        ),
        details: {
          grader: this.name,
          axes: axisResults,
          compositeScore,
          passThreshold: this.passThreshold,
          metrics,
          model: this.model,
          costUsd: response.total_cost_usd,
          numTurns: response.num_turns,
        },
      }
    } catch (error) {
      return {
        score: 0,
        pass: false,
        reasoning: `Performance grader error: ${error instanceof Error ? error.message : String(error)}`,
        details: { error: true, grader: this.name },
      }
    }
  }

  private async runAgent(
    systemPrompt: string,
    userPrompt: string,
    outputDir: string,
  ): Promise<AgentResult | null> {
    const taskId = outputDir.split('/').pop() ?? outputDir
    console.log(`Perf grader ${taskId}: Starting (model=${this.model})`)
    const startMs = Date.now()

    const agentPromise = (async (): Promise<AgentResult | null> => {
      let result: AgentResult | null = null
      let messageCount = 0

      for await (const message of query({
        prompt: userPrompt,
        options: {
          model: this.model,
          cwd: outputDir,
          systemPrompt,
          allowedTools: ['Read', 'Glob', 'Grep'],
          permissionMode: 'bypassPermissions',
          allowDangerouslySkipPermissions: true,
          maxTurns: this.maxTurns,
          maxBudgetUsd: this.maxBudgetUsd,
          outputFormat: {
            type: 'json_schema',
            schema: PERFORMANCE_EVAL_SCHEMA,
          },
          env: {
            ...process.env,
            CLAUDECODE: '',
          },
        },
      })) {
        messageCount++
        const elapsed = ((Date.now() - startMs) / 1000).toFixed(1)
        const msg = message as Record<string, unknown>

        if (message.type === 'result') {
          const res = message as AgentResult
          console.log(
            `Perf grader ${taskId}: Done (${elapsed}s, ${messageCount} msgs, subtype=${res.subtype}, cost=$${res.total_cost_usd?.toFixed(4)}, turns=${res.num_turns})`,
          )
          result = res
        } else if (message.type === 'assistant') {
          // Log tool calls the grader agent is making
          const content = msg.message as Record<string, unknown> | undefined
          const parts = (content?.content ?? []) as Array<
            Record<string, unknown>
          >
          const tools = parts
            .filter((p) => p.type === 'tool_use')
            .map((p) => {
              const input = p.input as Record<string, unknown> | undefined
              const path =
                input?.file_path ?? input?.pattern ?? input?.path ?? ''
              return `${p.name}(${String(path).split('/').pop() || ''})`
            })
          if (tools.length > 0) {
            console.log(
              `Perf grader ${taskId}: ${elapsed}s → ${tools.join(', ')}`,
            )
          }
        }
      }

      if (!result) {
        const elapsed = ((Date.now() - startMs) / 1000).toFixed(1)
        console.log(
          `Perf grader ${taskId}: Stream ended with no result (${elapsed}s, ${messageCount} msgs)`,
        )
      }
      return result
    })()

    let timeoutHandle: ReturnType<typeof setTimeout> | undefined
    const timeoutPromise = new Promise<never>((_, reject) => {
      timeoutHandle = setTimeout(() => {
        console.log(
          `Perf grader ${taskId}: Timeout after ${GRADER_TIMEOUT_MS / 1000}s`,
        )
        reject(
          new Error(
            `Performance grader timed out after ${GRADER_TIMEOUT_MS / 1000}s`,
          ),
        )
      }, GRADER_TIMEOUT_MS)
    })

    try {
      return await Promise.race([agentPromise, timeoutPromise])
    } finally {
      clearTimeout(timeoutHandle)
    }
  }

  private parseResponse(output: unknown): PerformanceEvalResponse | null {
    if (!output || typeof output !== 'object') return null

    const candidate = output as Record<string, unknown>
    if (!Array.isArray(candidate.axes)) return null

    const axes = candidate.axes
      .filter(
        (a: unknown): a is { axis: string; score: number; reasoning: string } =>
          typeof a === 'object' &&
          a !== null &&
          typeof (a as Record<string, unknown>).axis === 'string' &&
          typeof (a as Record<string, unknown>).score === 'number' &&
          typeof (a as Record<string, unknown>).reasoning === 'string',
      )
      .map((a) => ({
        axis: a.axis,
        score: Math.max(0, Math.min(100, a.score)),
        reasoning: a.reasoning,
      }))

    if (axes.length === 0) return null

    return { axes }
  }

  private buildReasoningSummary(
    response: PerformanceEvalResponse,
    composite: number,
    missingAxes: string[] = [],
  ): string {
    const lines = response.axes.map(
      (a) => `${a.axis}: ${a.score}/100 — ${a.reasoning}`,
    )
    if (missingAxes.length > 0) {
      lines.push(`\nMissing axes (scored as 0): ${missingAxes.join(', ')}`)
    }
    lines.push(`\nComposite: ${composite.toFixed(1)}/100`)
    return lines.join('\n')
  }
}

type AgentResult = {
  type: 'result'
  subtype: string
  result: string
  total_cost_usd: number
  num_turns: number
  structured_output?: unknown
}
