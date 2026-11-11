/**
 * The page's data: the task list and the chosen task's runs, from the viewer's API, kept current
 * by `/api/events`. Each event carries the new state of one task or run and its change number.
 *
 * The stream opens first and every (re)open takes fresh snapshots, so no change falls between a
 * snapshot and the stream. A snapshot can still land after events that are newer than it: the
 * page keeps the events since the stream opened and re-applies those numbered above the
 * snapshot's `version`.
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

/** Events kept for re-applying over a late snapshot; older ones are long since in any snapshot. */
const KEPT_EVENTS = 2000

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
  // Bumped each time the event stream opens; the snapshots below are taken after it.
  const [opened, setOpened] = useState(0)
  // The event handler outlives renders; it reads the chosen task from here.
  const taskIdRef = useRef<string | null>(null)
  taskIdRef.current = taskId
  const recent = useRef<ViewerEvent[]>([])

  useEffect(() => {
    getJson<ViewerInfo>(apiRoutes.info).then(setInfo, (cause) =>
      setError(String(cause)),
    )
  }, [])

  useEffect(() => {
    const events = new EventSource(apiRoutes.events)
    events.onopen = () => {
      recent.current = []
      setOpened((n) => n + 1)
    }
    events.onmessage = (message) => {
      const event = JSON.parse(message.data) as ViewerEvent
      recent.current.push(event)
      if (recent.current.length > KEPT_EVENTS) {
        recent.current.splice(0, recent.current.length - KEPT_EVENTS)
      }
      if (event.type === 'task') {
        setTasks((list) => upsert(list, event.task, newestFirst))
        if (taskIdRef.current === null) setTaskId(event.task.id)
      } else if (event.run.taskId === taskIdRef.current) {
        setRuns((list) => upsert(list, event.run, inOrder))
      }
    }
    return () => events.close()
  }, [])

  useEffect(() => {
    if (opened === 0) return
    getJson<TasksResponse>(apiRoutes.tasks).then(
      ({ tasks: snapshot, version }) => {
        let list = snapshot
        for (const event of recent.current) {
          if (event.version > version && event.type === 'task') {
            list = upsert(list, event.task, newestFirst)
          }
        }
        setTasks(list)
        setTaskId((current) => current ?? list[0]?.id ?? null)
      },
      (cause) => setError(String(cause)),
    )
  }, [opened])

  useEffect(() => {
    if (!taskId || opened === 0) return
    let current = true
    getJson<TaskRunsResponse>(apiRoutes.taskRuns(taskId)).then(
      ({ runs: snapshot, version }) => {
        if (!current) return
        let list = snapshot
        for (const event of recent.current) {
          if (
            event.version > version &&
            event.type === 'run' &&
            event.run.taskId === taskId
          ) {
            list = upsert(list, event.run, inOrder)
          }
        }
        setRuns(list)
      },
      (cause) => setError(String(cause)),
    )
    return () => {
      current = false
    }
  }, [taskId, opened])

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
