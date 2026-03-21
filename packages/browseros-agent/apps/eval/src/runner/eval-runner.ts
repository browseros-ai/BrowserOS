import { mkdir, writeFile } from 'node:fs/promises'
import { basename, dirname, join, resolve } from 'node:path'
import {
  dashboardState,
  setActiveExecutor,
  startDashboard,
  stopDashboard,
} from '../dashboard/server'
import type { ErrorSource, EvalConfig, Task } from '../types'
import {
  printValidationResult,
  validateConfig,
} from '../utils/config-validator'
import { ParallelExecutor } from './parallel-executor'
import {
  getTaskSourceDescription,
  loadTasks,
  TaskLoadError,
} from './task-loader'
import type {
  BatchSummary,
  GraderOptions,
  RunEvalOptions,
  TaskResult,
  TaskResultSummary,
  TaskSource,
} from './types'
import {
  getPrimaryGraderResult,
  isSuccessfulResult,
  resolveGraderOptions,
} from './types'

// ============================================================================
// Main Entry Point
// ============================================================================

export async function runEval(options: RunEvalOptions): Promise<void> {
  // Step 1: Validate configuration
  const config = await loadAndValidateConfig(options.configPath)

  // Step 2: Resolve paths relative to config location
  const configDir = dirname(resolve(options.configPath))
  const resolvedPaths = resolvePaths(options, config, configDir)

  // Log configuration
  console.log('Eval Configuration:')
  console.log(`  Config: ${options.configPath}`)
  console.log(`  Dataset: ${resolvedPaths.dataPath}`)
  console.log(`  Output: ${resolvedPaths.outputDir}`)
  console.log(`  Workers: ${config.num_workers}`)
  console.log(`  Agent: ${config.agent.type}`)
  console.log()

  // Step 3: Load tasks
  const taskSource = resolveTaskSource(options, resolvedPaths.dataPath)
  const { tasks } = await loadTasksWithLogging(taskSource)

  // Step 4: Setup
  await mkdir(resolvedPaths.outputDir, { recursive: true })
  const graderOptions = resolveGraderOptions(config)

  // Step 5: Start dashboard
  startDashboard({
    tasks,
    configName: options.configPath,
    agentType: config.agent.type,
    outputDir: resolvedPaths.outputDir,
  })

  // Step 6: Execute tasks (parallel or sequential based on num_workers)
  const results = await executeTasks(
    tasks,
    config,
    resolvedPaths.outputDir,
    graderOptions,
  )

  // Step 7: Summary
  const summary = buildSummary(results)
  await saveSummary(summary, resolvedPaths.outputDir)
  printSummary(summary)
  console.log(`\nResults saved to: ${resolvedPaths.outputDir}`)

  stopDashboard()
}

// ============================================================================
// Configuration
// ============================================================================

async function loadAndValidateConfig(configPath: string) {
  console.log('Validating configuration...')
  const validationResult = await validateConfig(configPath)
  printValidationResult(validationResult)

  if (!validationResult.valid || !validationResult.config) {
    throw new Error(
      'Configuration validation failed. Fix the above errors and try again.',
    )
  }

  return validationResult.config
}

interface ResolvedPaths {
  dataPath: string
  outputDir: string
}

function resolvePaths(
  options: RunEvalOptions,
  config: EvalConfig,
  configDir: string,
): ResolvedPaths {
  // Resolve dataset path: use options.dataPath if provided, otherwise resolve from config
  const dataPath = options.dataPath
    ? options.dataPath
    : config.dataset.startsWith('/')
      ? config.dataset
      : resolve(configDir, config.dataset)

  // Resolve output directory: results/{config-name}/{timestamp}/
  // Config name derived from config filename (e.g., "browseros-agent-weekly.json" → "browseros-agent-weekly")
  const configName = options.configPath
    ? basename(resolve(options.configPath), '.json')
    : 'eval'
  const timestamp = formatTimestamp(new Date())
  const resultsBase = config.output_dir
    ? config.output_dir.startsWith('/')
      ? config.output_dir
      : resolve(configDir, config.output_dir)
    : resolve(configDir, '..', 'results')
  const outputDir = join(resultsBase, configName, timestamp)

  return { dataPath, outputDir }
}

function formatTimestamp(date: Date): string {
  const y = date.getFullYear()
  const m = String(date.getMonth() + 1).padStart(2, '0')
  const d = String(date.getDate()).padStart(2, '0')
  const h = String(date.getHours()).padStart(2, '0')
  const min = String(date.getMinutes()).padStart(2, '0')
  return `${y}-${m}-${d}-${h}${min}`
}

// ============================================================================
// Task Loading
// ============================================================================

function resolveTaskSource(
  options: RunEvalOptions,
  dataPath: string,
): TaskSource {
  // If query is provided, use single task mode
  if (options.query) {
    return { type: 'single', query: options.query, startUrl: options.startUrl }
  }

  // Otherwise use file mode with the resolved dataPath
  return { type: 'file', path: dataPath }
}

async function loadTasksWithLogging(
  source: TaskSource,
): Promise<{ tasks: Awaited<ReturnType<typeof loadTasks>>['tasks'] }> {
  console.log(`Loading tasks from ${getTaskSourceDescription(source)}...`)

  try {
    const result = await loadTasks(source)
    console.log(`Loaded ${result.tasks.length} task(s)`)
    return { tasks: result.tasks }
  } catch (error) {
    if (error instanceof TaskLoadError) {
      throw new Error(`Failed to load tasks: ${error.message}`)
    }
    throw new Error(`Failed to load tasks: ${error}`)
  }
}

// ============================================================================
// Task Execution
// ============================================================================

async function executeTasks(
  tasks: Task[],
  config: EvalConfig,
  outputDir: string,
  graderOptions: GraderOptions | null,
): Promise<TaskResult[]> {
  console.log(`\n${'='.repeat(60)}`)
  console.log('STARTING EVALUATION')
  console.log(`${'='.repeat(60)}\n`)

  const numWorkers = config.num_workers || 1
  console.log(`Running with ${numWorkers} worker(s)`)
  if (config.restart_server_per_task) {
    console.log(`Server restart per task: enabled`)
  }
  console.log()

  const executor = new ParallelExecutor({
    numWorkers,
    config,
    outputDir,
    graderOptions,
    restartServerPerTask: config.restart_server_per_task,
    onEvent: (taskId, event) =>
      dashboardState.broadcastStreamEvent(taskId, event),
  })

  // Register so dashboard stop button works for CLI runs too
  setActiveExecutor(executor)
  try {
    return await executor.execute(tasks, (completed, total, task, result) => {
      printTaskProgress(completed, total, task, result)
    })
  } finally {
    setActiveExecutor(null)
  }
}

function printTaskProgress(
  completed: number,
  total: number,
  task: Task,
  result: TaskResult,
): void {
  const status =
    result.status === 'completed'
      ? 'DONE'
      : result.status === 'timeout'
        ? 'TIMEOUT'
        : 'FAILED'

  const duration =
    result.durationMs > 0 ? ` (${(result.durationMs / 1000).toFixed(1)}s)` : ''

  console.log(`[${completed}/${total}] ${task.query_id}: ${status}${duration}`)

  if (result.status === 'failed') {
    console.log(`  ERROR: ${result.error.message}`)
  } else if (isSuccessfulResult(result)) {
    for (const [name, gr] of Object.entries(result.graderResults)) {
      const icon = gr.pass ? 'PASS' : 'FAIL'
      console.log(`    ${name}: ${icon}`)
    }
  }
}

// ============================================================================
// Summary
// ============================================================================

function buildSummary(results: TaskResult[]): BatchSummary {
  // Track errors by source
  const errorsBySource: Partial<Record<ErrorSource, number>> = {}
  let totalWarnings = 0

  const taskSummaries: TaskResultSummary[] = results.map((r) => {
    let errorCount = 0
    let warningCount = 0
    let errorSources: ErrorSource[] | undefined
    let failureReason: string | undefined

    if (isSuccessfulResult(r)) {
      // Count errors and warnings from agent metadata
      errorCount = r.agentResult.metadata.errors?.length ?? 0
      warningCount = r.agentResult.metadata.warnings?.length ?? 0
      totalWarnings += warningCount

      // Track error sources
      if (r.agentResult.metadata.errors?.length) {
        errorSources = r.agentResult.metadata.errors.map((e) => e.source)
        for (const err of r.agentResult.metadata.errors) {
          errorsBySource[err.source] = (errorsBySource[err.source] ?? 0) + 1
        }
      }
    } else {
      // Failed task
      errorCount = 1
      errorSources = [r.errorSource]
      failureReason = r.error.message
      errorsBySource[r.errorSource] = (errorsBySource[r.errorSource] ?? 0) + 1
    }

    return {
      queryId: r.task.query_id,
      status: r.status,
      durationMs: r.durationMs,
      graderResults: isSuccessfulResult(r)
        ? Object.fromEntries(
            Object.entries(r.graderResults).map(([name, gr]) => [
              name,
              { pass: gr.pass, score: gr.score },
            ]),
          )
        : undefined,
      errorCount,
      warningCount,
      errorSources: errorSources?.length ? errorSources : undefined,
      failureReason,
    }
  })

  const completed = results.filter((r) => r.status === 'completed').length
  const timeout = results.filter((r) => r.status === 'timeout').length
  const failed = results.filter((r) => r.status === 'failed').length

  // Calculate pass rate using primary grader (fallback order)
  let totalGraded = 0
  let totalPasses = 0

  for (const result of results) {
    if (isSuccessfulResult(result)) {
      const primary = getPrimaryGraderResult(result.graderResults)
      if (primary) {
        totalGraded++
        if (primary.pass) totalPasses++
      }
    }
  }

  const passRate = totalGraded > 0 ? totalPasses / totalGraded : 0

  // Calculate average duration for non-failed tasks
  const durations = results
    .filter((r) => r.status !== 'failed')
    .map((r) => r.durationMs)
  const avgDurationMs =
    durations.length > 0
      ? durations.reduce((a, b) => a + b, 0) / durations.length
      : 0

  return {
    total: results.length,
    completed,
    failed,
    timeout,
    passRate,
    avgDurationMs,
    errorsBySource,
    totalWarnings,
    results: taskSummaries,
  }
}

async function saveSummary(
  summary: BatchSummary,
  outputDir: string,
): Promise<void> {
  await writeFile(
    join(outputDir, 'summary.json'),
    JSON.stringify(summary, null, 2),
  )
}

function printSummary(summary: BatchSummary): void {
  console.log('='.repeat(60))
  console.log('EVALUATION COMPLETE')
  console.log('='.repeat(60))
  console.log(`Total: ${summary.total} tasks`)
  console.log(`  Completed: ${summary.completed}`)
  console.log(`  Timeout: ${summary.timeout}`)
  console.log(`  Failed: ${summary.failed}`)
  console.log(`  Pass Rate: ${(summary.passRate * 100).toFixed(1)}%`)
  console.log(`  Avg Duration: ${(summary.avgDurationMs / 1000).toFixed(1)}s`)
}
