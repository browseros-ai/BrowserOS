/**
 * The page's data: the task list and the chosen task's runs, fetched from the viewer's API and
 * kept current by `/api/events`. Each event carries the new state of one task or run, so the
 * page applies it in place; after the event stream reconnects it refetches, since events sent
 * while it was down are not replayed.
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import type {
  RunDetail,
  RunResponse,
  RunSummary,
  TaskRunsResponse,
  TaskSummary,
  TasksResponse,
  ViewerEvent,
  ViewerInfo,
} from '../shared/api'
import { apiRoutes } from '../shared/api'

async function getJson<T>(path: string): Promise<T> {
  const response = await fetch(path)
  if (!response.ok) throw new Error(`${path}: HTTP ${response.status}`)
  return (await response.json()) as T
}

function upsert<T extends { id: string }>(
  items: T[],
  item: T,
  order: (a: T, b: T) => number,
): T[] {
  const index = items.findIndex((candidate) => candidate.id === item.id)
  const next =
    index === -1
      ? [...items, item]
      : items.map((candidate, i) => (i === index ? item : candidate))
  return next.sort(order)
}

const newestFirst = (a: TaskSummary, b: TaskSummary) =>
  b.createdAt.localeCompare(a.createdAt)
const inOrder = (a: RunSummary, b: RunSummary) => a.seq - b.seq

export interface ViewerState {
  info: ViewerInfo | null
  tasks: TaskSummary[]
  taskId: string | null
  selectTask: (taskId: string) => void
  runs: RunSummary[]
  error: string | null
}

export function useViewer(): ViewerState {
  const [info, setInfo] = useState<ViewerInfo | null>(null)
  const [tasks, setTasks] = useState<TaskSummary[]>([])
  const [taskId, setTaskId] = useState<string | null>(null)
  const [runs, setRuns] = useState<RunSummary[]>([])
  const [error, setError] = useState<string | null>(null)
  const [generation, setGeneration] = useState(0)
  // The event handler outlives renders; it reads the chosen task from here.
  const taskIdRef = useRef<string | null>(null)
  taskIdRef.current = taskId

  useEffect(() => {
    getJson<ViewerInfo>(apiRoutes.info).then(setInfo, (cause) =>
      setError(String(cause)),
    )
  }, [])

  // biome-ignore lint/correctness/useExhaustiveDependencies: `generation` is the refetch trigger.
  useEffect(() => {
    getJson<TasksResponse>(apiRoutes.tasks).then(
      ({ tasks: list }) => {
        setTasks(list)
        setTaskId((current) => current ?? list[0]?.id ?? null)
      },
      (cause) => setError(String(cause)),
    )
  }, [generation])

  // biome-ignore lint/correctness/useExhaustiveDependencies: `generation` is the refetch trigger.
  useEffect(() => {
    if (!taskId) return
    let current = true
    getJson<TaskRunsResponse>(apiRoutes.taskRuns(taskId)).then(
      (found) => {
        if (current) setRuns(found.runs)
      },
      (cause) => setError(String(cause)),
    )
    return () => {
      current = false
    }
  }, [taskId, generation])

  useEffect(() => {
    const events = new EventSource(apiRoutes.events)
    let dropped = false
    events.onerror = () => {
      dropped = true
    }
    events.onopen = () => {
      if (dropped) setGeneration((n) => n + 1)
      dropped = false
    }
    events.onmessage = (message) => {
      const event = JSON.parse(message.data) as ViewerEvent
      if (event.type === 'task') {
        setTasks((list) => upsert(list, event.task, newestFirst))
        if (taskIdRef.current === null) setTaskId(event.task.id)
      } else if (event.run.taskId === taskIdRef.current) {
        setRuns((list) => upsert(list, event.run, inOrder))
      }
    }
    return () => events.close()
  }, [])

  const selectTask = useCallback((next: string) => {
    setRuns([])
    setTaskId(next)
  }, [])

  return { info, tasks, taskId, selectTask, runs, error }
}

/** One run in full, refetched whenever its summary changes (it was still running). */
export function useRunDetail(run: RunSummary, open: boolean): RunDetail | null {
  const [detail, setDetail] = useState<RunDetail | null>(null)
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new status means a new reply to fetch.
  useEffect(() => {
    if (!open) return
    let current = true
    getJson<RunResponse>(apiRoutes.run(run.id)).then(
      (found) => {
        if (current) setDetail(found.run)
      },
      () => {},
    )
    return () => {
      current = false
    }
  }, [run.id, run.status, open])
  return detail
}
