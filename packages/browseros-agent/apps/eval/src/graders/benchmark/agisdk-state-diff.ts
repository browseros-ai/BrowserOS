import { spawn } from 'node:child_process'
import { join } from 'node:path'
import type { GraderResult } from '../../types'
import { callMcpTool } from '../../utils/mcp-client'
import type { Grader, GraderInput } from '../types'

const EVAL_SCRIPT = join(
  import.meta.dirname,
  '..',
  '..',
  '..',
  'scripts',
  'agisdk-evaluate.py',
)

export class AgisdkStateDiffGrader implements Grader {
  name = 'agisdk_state_diff'

  async grade(input: GraderInput): Promise<GraderResult> {
    const taskId = this.extractTaskId(input.task.query_id)
    const startUrl = this.extractStartUrl(input)
    const mcpEndpoint =
      input.mcpUrl ||
      `${process.env.BROWSEROS_SERVER_URL || 'http://127.0.0.1:9110'}/mcp`

    if (!startUrl) {
      return {
        score: 0,
        pass: false,
        reasoning: 'Could not determine clone site URL from task',
      }
    }

    const origin = new URL(startUrl).origin

    let envState: Record<string, unknown>
    try {
      envState = await this.fetchFinishState(origin, mcpEndpoint)
    } catch (error) {
      return {
        score: 0,
        pass: false,
        reasoning: `Failed to fetch /finish endpoint: ${error instanceof Error ? error.message : String(error)}`,
        details: { origin, error: true },
      }
    }

    try {
      const result = await this.runPythonEvaluator(
        taskId,
        envState,
        input.finalAnswer || '',
      )
      return {
        score: result.reward,
        pass: result.pass,
        reasoning:
          result.message ||
          (result.pass ? 'All criteria passed' : 'Some criteria failed'),
        details: {
          reward: result.reward,
          per_criterion: result.per_criterion,
          origin,
          agisdk_task_id: taskId,
        },
      }
    } catch (error) {
      return {
        score: 0,
        pass: false,
        reasoning: `Python evaluator error: ${error instanceof Error ? error.message : String(error)}`,
        details: { error: true },
      }
    }
  }

  private extractTaskId(queryId: string): string {
    return queryId.replace(/^agisdk-/, '')
  }

  private extractStartUrl(input: GraderInput): string | null {
    // Derive from task_id: "dashdish-10" → "https://evals-dashdish.vercel.app"
    // Task IDs are "{site}-{number}" where site may contain hyphens (e.g. "fly-unified-5")
    const taskId = this.extractTaskId(input.task.query_id)
    const siteId = taskId.replace(/-\d+$/, '')
    if (siteId) return `https://evals-${siteId}.vercel.app`

    // Fallback: search messages for vercel.app URLs
    for (const msg of input.messages) {
      const text =
        msg.type === 'user'
          ? msg.content
          : msg.type === 'tool-input-available'
            ? JSON.stringify(msg.input)
            : ''
      const urlMatch = text.match(/https?:\/\/[^\s"']+\.vercel\.app/)
      if (urlMatch) return urlMatch[0]
    }

    return null
  }

  private async fetchFinishState(
    origin: string,
    mcpEndpoint: string,
  ): Promise<Record<string, unknown>> {
    const finishUrl = `${origin}/finish`

    // Navigate browser to /finish page (state diff is rendered client-side)
    await callMcpTool(mcpEndpoint, 'navigate_page', {
      url: finishUrl,
      page: 1,
    })

    // Wait for the page to render, then extract JSON from <pre> element
    const result = await callMcpTool(mcpEndpoint, 'evaluate_script', {
      page: 1,
      expression: `
        new Promise((resolve, reject) => {
          let attempts = 0;
          const check = () => {
            const pre = document.querySelector('pre');
            if (pre && pre.textContent.trim().startsWith('{')) {
              resolve(pre.textContent);
            } else if (++attempts > 20) {
              reject(new Error('Timed out waiting for <pre> JSON on /finish'));
            } else {
              setTimeout(check, 500);
            }
          };
          check();
        })
      `,
    })

    const textContent = result.content?.find(
      (c: { type: string }) => c.type === 'text',
    )
    if (!textContent?.text) {
      throw new Error('No text content returned from /finish page')
    }

    return JSON.parse(textContent.text) as Record<string, unknown>
  }

  private runPythonEvaluator(
    taskId: string,
    envState: Record<string, unknown>,
    modelResponse: string,
  ): Promise<{
    reward: number
    pass: boolean
    message: string
    per_criterion: unknown[]
  }> {
    return new Promise((resolve, reject) => {
      const proc = spawn('python3', [EVAL_SCRIPT], {
        stdio: ['pipe', 'pipe', 'pipe'],
      })

      const inputData = JSON.stringify({
        task_id: taskId,
        env_state: envState,
        model_response: modelResponse,
      })

      let stdout = ''
      let stderr = ''

      proc.stdout.on('data', (data: Buffer) => {
        stdout += data.toString()
      })

      proc.stderr.on('data', (data: Buffer) => {
        stderr += data.toString()
      })

      proc.on('close', (code) => {
        if (code !== 0) {
          reject(
            new Error(`Python evaluator exited with code ${code}: ${stderr}`),
          )
          return
        }

        try {
          const result = JSON.parse(stdout.trim())
          resolve(result)
        } catch {
          reject(new Error(`Failed to parse evaluator output: ${stdout}`))
        }
      })

      proc.on('error', (err) => {
        reject(new Error(`Failed to spawn Python evaluator: ${err.message}`))
      })

      proc.stdin.write(inputData)
      proc.stdin.end()
    })
  }
}
