import { join, resolve } from 'node:path'
import type { GraderResult } from '../../types'
import type { Grader, GraderInput } from '../types'

interface InfinityEvalInput {
  app_server_url: string
  verifier_path: string
  task_id: string
}

interface InfinityEvalOutput {
  pass: boolean
  reward: number
  message: string
}

const EVAL_SCRIPT = resolve(
  import.meta.dir,
  '../../../scripts/infinity-evaluate.py',
)

export class InfinityStateGrader implements Grader {
  name = 'infinity_state'

  async grade(input: GraderInput): Promise<GraderResult> {
    const parsed = this.parseQueryId(input.task.query_id)
    if (!parsed) {
      return {
        score: 0,
        pass: false,
        reasoning: `Cannot parse query_id "${input.task.query_id}" — expected format: infinity-{app}-{task_id}`,
      }
    }

    const appServerUrl = this.resolveAppServerUrl(input)
    if (!appServerUrl) {
      return {
        score: 0,
        pass: false,
        reasoning: 'Cannot determine app server URL',
      }
    }

    const infinityDir = process.env.WEBARENA_INFINITY_DIR
    if (!infinityDir) {
      return {
        score: 0,
        pass: false,
        reasoning:
          'WEBARENA_INFINITY_DIR env var not set. Point it to the webarena-infinity repo root.',
      }
    }

    const verifierPath = join(
      infinityDir,
      'apps',
      parsed.appName,
      'real-tasks',
      `${parsed.taskId}.py`,
    )

    const evalInput: InfinityEvalInput = {
      app_server_url: appServerUrl,
      verifier_path: verifierPath,
      task_id: input.task.query_id,
    }

    try {
      const result = await this.runPythonEvaluator(evalInput)
      return {
        score: result.pass ? 1 : 0,
        pass: result.pass,
        reasoning: result.message,
        details: {
          reward: result.reward,
          app_name: parsed.appName,
          app_server_url: appServerUrl,
        },
      }
    } catch (error) {
      return {
        score: 0,
        pass: false,
        reasoning: `Evaluator process error: ${error instanceof Error ? error.message : String(error)}`,
      }
    }
  }

  private parseQueryId(
    queryId: string,
  ): { appName: string; taskId: string } | null {
    // Task IDs start with "task_", app names may contain hyphens
    // e.g. "infinity-elation-prescriptions-task_h69"
    const match = queryId.match(/^infinity-(.+)-(task_.+)$/)
    if (!match) return null
    return { appName: match[1], taskId: match[2] }
  }

  private resolveAppServerUrl(input: GraderInput): string | null {
    // Passed directly from task executor (started by InfinityAppManager)
    if (input.infinityAppUrl) return input.infinityAppUrl

    // Fallback: env var for manual testing
    if (process.env.INFINITY_APP_URL) return process.env.INFINITY_APP_URL

    return null
  }

  private async runPythonEvaluator(
    evalInput: InfinityEvalInput,
  ): Promise<InfinityEvalOutput> {
    const proc = Bun.spawn(['python3', EVAL_SCRIPT], {
      stdin: 'pipe',
      stdout: 'pipe',
      stderr: 'pipe',
    })

    const inputJson = JSON.stringify(evalInput)
    proc.stdin.write(inputJson)
    proc.stdin.end()

    const stdout = await new Response(proc.stdout).text()
    const stderr = await new Response(proc.stderr).text()
    const exitCode = await proc.exited

    if (exitCode !== 0) {
      throw new Error(
        `Python evaluator exited with code ${exitCode}: ${stderr || stdout}`,
      )
    }

    return JSON.parse(stdout.trim()) as InfinityEvalOutput
  }
}
