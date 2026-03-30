/**
 * Parallel Executor
 *
 * Each worker gets its own isolated BrowserOS stack:
 *   - BrowserOSAppManager (Chrome + Server on unique ports)
 *   - TaskExecutor (uses that worker's server URL)
 *
 * Port allocation: Worker N → CDP=base+N, Server=base+N, Extension=base+N
 */

import type { EvalConfig, Task } from '../types'
import type { EvalPorts } from '../utils/dev-config'
import { BrowserOSAppManager } from './browseros-app-manager'
import { createTaskExecutor } from './task-executor'
import type { GraderOptions, TaskResult } from './types'

// ============================================================================
// Types
// ============================================================================

export interface ParallelExecutorConfig {
  numWorkers: number
  config: EvalConfig
  outputDir: string
  graderOptions: GraderOptions | null
  restartServerPerTask?: boolean
  onEvent?: (taskId: string, event: Record<string, unknown>) => void
}

export type ProgressCallback = (
  completed: number,
  total: number,
  task: Task,
  result: TaskResult,
) => void

// ============================================================================
// Task Queue (thread-safe for single-threaded async — index is atomic)
// ============================================================================

class TaskQueue {
  private tasks: Task[]
  private index: number = 0
  private stopped: boolean = false

  constructor(tasks: Task[]) {
    this.tasks = [...tasks]
  }

  next(): Task | null {
    if (this.stopped || this.index >= this.tasks.length) return null
    return this.tasks[this.index++]
  }

  stop(): void {
    this.stopped = true
  }
}

// ============================================================================
// Parallel Executor
// ============================================================================

export class ParallelExecutor {
  private readonly numWorkers: number
  private readonly appManagers = new Map<number, BrowserOSAppManager>()
  private completedCount: number = 0
  private readonly resultLock = new Map<string, TaskResult>()
  private queue: TaskQueue | null = null

  constructor(private readonly config: ParallelExecutorConfig) {
    this.numWorkers = Math.max(1, config.numWorkers)
  }

  async stop(): Promise<void> {
    console.log('\nStopping eval run...')
    this.queue?.stop()
    const kills = [...this.appManagers.values()].map((m) => m.killApp())
    await Promise.allSettled(kills)
  }

  async execute(
    tasks: Task[],
    onProgress?: ProgressCallback,
  ): Promise<TaskResult[]> {
    if (tasks.length === 0) return []

    const cleanup = this.setupSignalHandlers()

    const loadExtensions = this.config.config.browseros.load_extensions ?? false

    // Patch NopeCHA API key before launching any workers
    const captchaConfig = this.config.config.captcha
    if (captchaConfig) {
      const apiKey = process.env[captchaConfig.api_key_env]
      if (apiKey) {
        BrowserOSAppManager.patchNopechaApiKey(apiKey)
      }
    }

    this.queue = new TaskQueue(tasks)
    const totalTasks = tasks.length

    try {
      const queue = this.queue
      // Launch N workers in parallel — each gets its own Chrome + Server
      const workers = Array.from({ length: this.numWorkers }, (_, i) =>
        this.runWorker(i, queue, totalTasks, loadExtensions, onProgress),
      )
      await Promise.all(workers)

      // Return results in original task order
      return tasks.map((task) => {
        const result = this.resultLock.get(task.query_id)
        if (!result) {
          return {
            status: 'failed' as const,
            task,
            error: new Error('Task result not found'),
            errorSource: 'unknown' as const,
            durationMs: 0,
          }
        }
        return result
      })
    } finally {
      cleanup()
    }
  }

  private async runWorker(
    workerIndex: number,
    queue: TaskQueue,
    totalTasks: number,
    loadExtensions: boolean,
    onProgress?: ProgressCallback,
  ): Promise<void> {
    // Per-worker isolated ports
    const basePorts: EvalPorts = {
      cdp: this.config.config.browseros.base_cdp_port,
      server: this.config.config.browseros.base_server_port,
      extension: this.config.config.browseros.base_extension_port,
    }
    const headless = this.config.config.browseros.headless ?? false
    const appManager = new BrowserOSAppManager(
      workerIndex,
      basePorts,
      loadExtensions,
      headless,
    )
    this.appManagers.set(workerIndex, appManager)

    // Per-worker executor pointing to this worker's server
    const workerConfig: typeof this.config.config = {
      ...this.config.config,
      browseros: {
        ...this.config.config.browseros,
        server_url: appManager.getServerUrl(),
      },
    }
    const executor = createTaskExecutor(
      workerConfig,
      this.config.outputDir,
      this.config.graderOptions,
      this.config.onEvent,
    )

    try {
      // Always start Chrome+Server once for this worker
      console.log(`\n  Worker ${workerIndex}: Starting BrowserOS stack...`)
      await appManager.restart()

      while (true) {
        const task = queue.next()
        if (!task) break

        const taskStartTime = Date.now()
        let result: TaskResult

        try {
          // Restart between tasks if configured
          if (this.config.restartServerPerTask) {
            console.log(`\n${'─'.repeat(60)}`)
            console.log(`  Worker ${workerIndex}: Task: ${task.query_id}`)
            console.log(`${'─'.repeat(60)}`)
            await appManager.restart()
          }

          this.config.onEvent?.(task.query_id, {
            type: 'task-state',
            taskId: task.query_id,
            status: 'running',
          })
          result = await executor.execute(task)
          console.log(
            `  Worker ${workerIndex}: ${task.query_id}: ${result.status}`,
          )
        } catch (error) {
          console.error(
            `  Worker ${workerIndex}: ${task.query_id}: FAILED - ${error instanceof Error ? error.message : String(error)}`,
          )
          result = {
            status: 'failed',
            task,
            error: error instanceof Error ? error : new Error(String(error)),
            errorSource: 'unknown',
            durationMs: Date.now() - taskStartTime,
          }
        }

        this.resultLock.set(task.query_id, result)
        this.completedCount++

        // Emit task completion to dashboard
        const stateEvent: Record<string, unknown> = {
          type: 'task-state',
          taskId: task.query_id,
          status: result.status,
          durationMs: result.durationMs,
        }
        if (result.status !== 'failed' && 'graderResults' in result) {
          stateEvent.graderResults = Object.fromEntries(
            Object.entries(result.graderResults).map(([name, gr]) => [
              name,
              {
                pass: gr.pass,
                score: gr.score,
                reasoning: gr.reasoning,
                details: gr.details,
              },
            ]),
          )
          stateEvent.screenshotCount =
            result.agentResult?.metadata?.total_steps ?? 0
        }
        this.config.onEvent?.(task.query_id, stateEvent)

        onProgress?.(this.completedCount, totalTasks, task, result)

        if (this.config.restartServerPerTask) {
          await new Promise((resolve) => setTimeout(resolve, 2000))
        }
      }
    } finally {
      await appManager.killApp()
    }
  }

  /**
   * SIGINT/SIGTERM kills all Chrome + Server instances across all workers.
   * Returns a cleanup function that removes the listeners after execute() completes.
   */
  private setupSignalHandlers(): () => void {
    const onSignal = async () => {
      console.log('\nShutting down all workers...')
      this.queue?.stop()
      const kills = [...this.appManagers.values()].map((m) => m.killApp())
      await Promise.allSettled(kills)
      process.exit(0)
    }
    process.on('SIGINT', onSignal)
    process.on('SIGTERM', onSignal)
    return () => {
      process.off('SIGINT', onSignal)
      process.off('SIGTERM', onSignal)
    }
  }
}

// ============================================================================
// Factory
// ============================================================================

export async function executeTasksInParallel(
  tasks: Task[],
  config: ParallelExecutorConfig,
  onProgress?: ProgressCallback,
): Promise<TaskResult[]> {
  const executor = new ParallelExecutor(config)
  return executor.execute(tasks, onProgress)
}
