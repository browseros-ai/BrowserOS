import { join } from 'node:path'
import { createAgent } from '../agents'
import type { AgentContext, AgentResult } from '../agents/types'
import { CaptureContext } from '../capture/context'
import {
  hasExistingGraderResults,
  TrajectorySaver,
} from '../capture/trajectory-saver'
import { runGraders } from '../graders/registry'
import type { ErrorSource, EvalConfig, GraderResult, Task } from '../types'
import { callMcpTool } from '../utils/mcp-client'
import { InfinityAppManager } from './infinity-app-manager'
import type { GraderOptions, TaskResult } from './types'

// ============================================================================
// Errors
// ============================================================================

export class TaskExecutionError extends Error {
  public readonly errorSource: ErrorSource

  constructor(
    message: string,
    public readonly task: Task,
    public readonly phase:
      | 'navigation'
      | 'agent_execution'
      | 'grading'
      | 'cleanup',
    public readonly cause?: Error,
  ) {
    super(message)
    this.name = 'TaskExecutionError'
    this.errorSource = phase as ErrorSource
  }
}

// ============================================================================
// Task Executor
// ============================================================================

export interface TaskExecutorDeps {
  graderOptions: GraderOptions | null
  onEvent?: (taskId: string, event: Record<string, unknown>) => void
}

export class TaskExecutor {
  constructor(
    private readonly config: EvalConfig,
    private readonly outputDir: string,
    private readonly deps: TaskExecutorDeps,
  ) {}

  /**
   * Resolve the initial page ID via list_pages MCP call.
   * Called once per task on a fresh browser — there's exactly one page.
   */
  private async resolveInitialPageId(mcpUrl: string): Promise<number> {
    try {
      const result = await callMcpTool(mcpUrl, 'list_pages', {})
      if (!result.isError) {
        const textContent = result.content?.find(
          (c: { type: string }) => c.type === 'text',
        )
        const match = textContent?.text?.match(/^\s*(\d+)\./m)
        if (match) return Number.parseInt(match[1], 10)
      }
    } catch {
      // Fall through to default
    }
    // Fresh browser always has page 1
    return 1
  }

  async execute(task: Task): Promise<TaskResult> {
    const startTime = Date.now()
    const mcpUrl = `${this.config.browseros.server_url}/mcp`

    // Check if task already has grader results (resume capability)
    const existing = await hasExistingGraderResults(
      this.outputDir,
      task.query_id,
    )
    if (existing.exists && existing.metadata) {
      console.log(`  Skipping: already has grader results`)
      return {
        status:
          existing.metadata.termination_reason === 'timeout'
            ? 'timeout'
            : 'completed',
        task,
        agentResult: {
          metadata: existing.metadata,
          messages: [],
          finalAnswer: existing.metadata.final_answer,
        },
        graderResults: existing.metadata.grader_results,
        durationMs: existing.metadata.total_duration_ms,
      }
    }

    // Resolve page ID once — fresh browser has exactly one page
    const pageId = await this.resolveInitialPageId(mcpUrl)

    // For Infinity tasks, start a fresh app server per task
    let infinityManager: InfinityAppManager | null = null
    let actualStartUrl = task.start_url

    if (task.dataset === 'webarena-infinity') {
      const appName = (task.metadata?.additional as Record<string, unknown>)
        ?.app_name as string
      const appBasePort =
        ((task.metadata?.additional as Record<string, unknown>)
          ?.app_base_port as number) || 8000
      const workerIndex = this.config.browseros.base_server_port - 9110 // derive from port offset

      if (appName && process.env.WEBARENA_INFINITY_DIR) {
        infinityManager = new InfinityAppManager(workerIndex, appBasePort)
        try {
          actualStartUrl = await infinityManager.startApp(appName)
          console.log(
            `  Infinity app "${appName}" started on port ${infinityManager.getPort()}`,
          )
        } catch (error) {
          throw new TaskExecutionError(
            `Failed to start Infinity app: ${error instanceof Error ? error.message : String(error)}`,
            task,
            'navigation',
            error instanceof Error ? error : undefined,
          )
        }
      }
    }

    try {
      // Phase 1: Set viewport + navigate to start URL
      try {
        await callMcpTool(mcpUrl, 'evaluate_script', {
          page: pageId,
          expression: 'window.resizeTo(1440, 900)',
        })
      } catch (vpError) {
        console.warn(
          `  Viewport resize failed: ${vpError instanceof Error ? vpError.message : String(vpError)}`,
        )
      }

      if (actualStartUrl && actualStartUrl !== 'about:blank') {
        try {
          await callMcpTool(mcpUrl, 'navigate_page', {
            url: actualStartUrl,
            page: pageId,
          })
        } catch (error) {
          throw new TaskExecutionError(
            `Failed to navigate to start URL: ${error instanceof Error ? error.message : String(error)}`,
            task,
            'navigation',
            error instanceof Error ? error : undefined,
          )
        }
      }

      // Phase 2: Execute agent
      const agentResult = await this.executeAgent(task, pageId)

      // Phase 3: Run graders
      const graderResults = await this.runGraders(
        task,
        agentResult,
        infinityManager?.getUrl(),
      )

      const status =
        agentResult.metadata.termination_reason === 'timeout'
          ? 'timeout'
          : 'completed'

      return {
        status,
        task,
        agentResult,
        graderResults,
        durationMs: Date.now() - startTime,
      }
    } catch (error) {
      const errorSource: ErrorSource =
        error instanceof TaskExecutionError ? error.errorSource : 'unknown'

      return {
        status: 'failed',
        task,
        error: error instanceof Error ? error : new Error(String(error)),
        errorSource,
        durationMs: Date.now() - startTime,
      }
    } finally {
      // Navigate to about:blank to clean up
      try {
        await callMcpTool(mcpUrl, 'navigate_page', {
          url: 'about:blank',
          page: pageId,
        })
      } catch {
        // Ignore cleanup errors
      }

      // Stop Infinity app server if running
      if (infinityManager) {
        await infinityManager.stop().catch(() => {})
      }
    }
  }

  private async executeAgent(task: Task, pageId: number): Promise<AgentResult> {
    try {
      const { capture, taskOutputDir } = await CaptureContext.create({
        serverUrl: this.config.browseros.server_url,
        outputDir: this.outputDir,
        taskId: task.query_id,
        initialPageId: pageId,
        onEvent: this.deps.onEvent,
      })

      const context: AgentContext = {
        config: this.config,
        task,
        initialPageId: pageId,
        outputDir: this.outputDir,
        taskOutputDir,
        capture,
      }

      const agent = createAgent(context)
      return await agent.execute()
    } catch (error) {
      if (error instanceof TaskExecutionError) {
        throw error
      }
      throw new TaskExecutionError(
        `Agent execution failed: ${error instanceof Error ? error.message : String(error)}`,
        task,
        'agent_execution',
        error instanceof Error ? error : undefined,
      )
    }
  }

  private async runGraders(
    task: Task,
    agentResult: AgentResult,
    infinityAppUrl?: string,
  ): Promise<Record<string, GraderResult>> {
    const configGraders = this.config.graders ?? []
    const taskGraders = task.graders ?? []
    const graderNames = configGraders.length > 0 ? configGraders : taskGraders
    if (graderNames.length === 0) {
      return {}
    }

    try {
      const graderResults = await runGraders(
        graderNames,
        {
          task: {
            query_id: task.query_id,
            query: task.query,
            dataset: task.dataset,
          },
          messages: agentResult.messages,
          screenshotCount:
            agentResult.metadata.screenshot_count ??
            agentResult.metadata.total_steps,
          finalAnswer: agentResult.finalAnswer,
          expectedAnswer: (task.metadata?.additional as Record<string, unknown>)
            ?.answer as string | undefined,
          outputDir: join(this.outputDir, task.query_id),
          mcpUrl: `${this.config.browseros.server_url}/mcp`,
          infinityAppUrl,
        },
        this.deps.graderOptions,
      )

      try {
        const saver = new TrajectorySaver(this.outputDir, task.query_id)
        await saver.updateGraderResults(graderResults)
      } catch (saveError) {
        console.warn(
          `  Failed to persist grader results: ${saveError instanceof Error ? saveError.message : String(saveError)}`,
        )
      }

      return graderResults
    } catch (error) {
      console.warn(
        `  Grading failed: ${error instanceof Error ? error.message : String(error)}`,
      )
      return {
        _error: {
          score: 0,
          pass: false,
          reasoning: `Grading failed: ${error instanceof Error ? error.message : String(error)}`,
        },
      }
    }
  }
}

// ============================================================================
// Factory
// ============================================================================

export function createTaskExecutor(
  config: EvalConfig,
  outputDir: string,
  graderOptions: GraderOptions | null,
  onEvent?: (taskId: string, event: Record<string, unknown>) => void,
): TaskExecutor {
  return new TaskExecutor(config, outputDir, {
    graderOptions,
    onEvent,
  })
}
