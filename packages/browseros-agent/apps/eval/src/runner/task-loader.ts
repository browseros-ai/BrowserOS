import { readFile } from 'node:fs/promises'
import { z } from 'zod'
import { type Task, TaskSchema } from '../types'
import type { TaskLoadResult, TaskSource } from './types'

// ============================================================================
// Errors
// ============================================================================

export class TaskLoadError extends Error {
  constructor(
    message: string,
    public readonly source: TaskSource,
    public readonly cause?: Error,
  ) {
    super(message)
    this.name = 'TaskLoadError'
  }
}

export class TaskValidationError extends Error {
  constructor(
    message: string,
    public readonly lineNumber: number,
    public readonly validationErrors: z.ZodError,
  ) {
    super(message)
    this.name = 'TaskValidationError'
  }
}

// ============================================================================
// Task Loader
// ============================================================================

export async function loadTasks(source: TaskSource): Promise<TaskLoadResult> {
  if (source.type === 'file') {
    return loadTasksFromFile(source.path)
  }

  return createSingleTask(source.query, source.startUrl)
}

async function loadTasksFromFile(filePath: string): Promise<TaskLoadResult> {
  const source: TaskSource = { type: 'file', path: filePath }

  let content: string
  try {
    content = await readFile(filePath, 'utf-8')
  } catch (error) {
    throw new TaskLoadError(
      `Failed to read tasks file: ${filePath}`,
      source,
      error instanceof Error ? error : undefined,
    )
  }

  const lines = content
    .trim()
    .split('\n')
    .filter((line) => line.trim().length > 0)

  if (lines.length === 0) {
    throw new TaskLoadError('Tasks file is empty', source)
  }

  const tasks: Task[] = []
  const errors: Array<{ line: number; error: string }> = []

  for (let i = 0; i < lines.length; i++) {
    const lineNumber = i + 1
    const line = lines[i]

    try {
      const parsed = JSON.parse(line)
      const validated = TaskSchema.parse(parsed)
      tasks.push(validated)
    } catch (error) {
      if (error instanceof SyntaxError) {
        errors.push({
          line: lineNumber,
          error: `Invalid JSON: ${error.message}`,
        })
      } else if (error instanceof z.ZodError) {
        const issues = error.issues
          .map((i) => `${i.path.join('.')}: ${i.message}`)
          .join(', ')
        errors.push({
          line: lineNumber,
          error: `Validation failed: ${issues}`,
        })
      } else {
        errors.push({
          line: lineNumber,
          error: `Unknown error: ${String(error)}`,
        })
      }
    }
  }

  if (errors.length > 0) {
    const errorSummary = errors
      .slice(0, 5)
      .map(
        (e: { line: number; error: string }) => `  Line ${e.line}: ${e.error}`,
      )
      .join('\n')

    const moreErrors =
      errors.length > 5 ? `\n  ... and ${errors.length - 5} more errors` : ''

    throw new TaskLoadError(
      `Failed to parse ${errors.length} task(s):\n${errorSummary}${moreErrors}`,
      source,
    )
  }

  // Validate unique query_ids
  const seenIds = new Set<string>()
  const duplicates: string[] = []
  for (const task of tasks) {
    if (seenIds.has(task.query_id)) {
      duplicates.push(task.query_id)
    }
    seenIds.add(task.query_id)
  }
  if (duplicates.length > 0) {
    throw new TaskLoadError(
      `Duplicate query_ids found: ${duplicates.join(', ')}`,
      source,
    )
  }

  return { tasks, source }
}

function createSingleTask(query: string, startUrl?: string): TaskLoadResult {
  const source: TaskSource = { type: 'single', query, startUrl }

  if (!query.trim()) {
    throw new TaskLoadError('Query cannot be empty', source)
  }

  const task: Task = {
    query_id: `single-${Date.now()}`,
    dataset: 'manual',
    query: query.trim(),
    graders: ['performance_grader'],
    start_url: startUrl,
    metadata: {
      original_task_id: 'manual',
    },
  }

  return { tasks: [task], source }
}

// ============================================================================
// Utilities
// ============================================================================

export function getTaskSourceDescription(source: TaskSource): string {
  if (source.type === 'file') {
    return `file: ${source.path}`
  }
  return 'single task mode'
}
