import type { AuditAgent } from '@browseros/claw-api'
import { agentIdentity } from '@/components/harness/harness.types'
import type { TaskSummary } from '@/modules/api/audit.hooks'

const DAY_MS = 24 * 60 * 60 * 1000

function startOfDay(ms: number): number {
  const day = new Date(ms)
  day.setHours(0, 0, 0, 0)
  return day.getTime()
}

function daysBefore(startedAt: number, now: number): number {
  return Math.round((startOfDay(now) - startOfDay(startedAt)) / DAY_MS)
}

export interface AgentOption {
  slug: string
  name: string
}

/**
 * The agents in the loaded tasks, shaped like the server's list: most tasks
 * first, named by each agent's latest task. The picker falls back to these
 * when the full list cannot be fetched.
 */
export function agentsInTasks(tasks: readonly TaskSummary[]): AuditAgent[] {
  const seen = new Map<string, AuditAgent>()
  for (const task of tasks) {
    const known = seen.get(task.slug)
    if (!known) {
      seen.set(task.slug, {
        slug: task.slug,
        label: task.label,
        taskCount: 1,
        lastStartedAt: task.startedAt,
      })
      continue
    }
    known.taskCount += 1
    if (task.startedAt > known.lastStartedAt) {
      known.lastStartedAt = task.startedAt
      known.label = task.label
    }
  }
  return [...seen.values()].sort(
    (left, right) =>
      right.taskCount - left.taskCount ||
      right.lastStartedAt - left.lastStartedAt,
  )
}

/**
 * Every agent in History, in the server's order (most tasks first), named the
 * way the task rows name them. A selected agent the server does not list is
 * kept so the picker can still show and clear it.
 */
export function agentOptionsFor(
  agents: readonly AuditAgent[],
  selectedSlug: string | null,
): AgentOption[] {
  const options = agents.map((agent) => ({
    slug: agent.slug,
    name: agentIdentity(undefined, agent.label || agent.slug).name,
  }))
  if (selectedSlug && !agents.some((agent) => agent.slug === selectedSlug)) {
    options.push({
      slug: selectedSlug,
      name: agentIdentity(undefined, selectedSlug).name,
    })
  }
  return options
}

const DAY_HEADING = new Intl.DateTimeFormat(undefined, {
  weekday: 'long',
  month: 'long',
  day: 'numeric',
})

/** "Today", "Yesterday", then "Wednesday, October 21". */
export function dayLabel(startedAt: number, now: number): string {
  const days = daysBefore(startedAt, now)
  if (days <= 0) return 'Today'
  if (days === 1) return 'Yesterday'
  return DAY_HEADING.format(new Date(startedAt))
}

export interface DayGroup {
  key: number
  label: string
  tasks: TaskSummary[]
}

/** Tasks newest first, split into one group per local calendar day. */
export function groupByDay(
  tasks: readonly TaskSummary[],
  now: number,
): DayGroup[] {
  const groups: DayGroup[] = []
  const ordered = [...tasks].sort(
    (left, right) => right.startedAt - left.startedAt,
  )
  for (const task of ordered) {
    const key = startOfDay(task.startedAt)
    const current = groups.at(-1)
    if (current?.key === key) {
      current.tasks.push(task)
      continue
    }
    groups.push({ key, label: dayLabel(task.startedAt, now), tasks: [task] })
  }
  return groups
}

/** "2:41 PM" in the reader's locale. */
export function taskTime(startedAt: number): string {
  return new Date(startedAt).toLocaleTimeString(undefined, {
    hour: 'numeric',
    minute: '2-digit',
  })
}

/** "2:41 PM" for today, "Yesterday", then "Oct 21" in the reader's locale. */
export function taskWhen(startedAt: number, now: number): string {
  const days = daysBefore(startedAt, now)
  if (days <= 0) return taskTime(startedAt)
  if (days === 1) return 'Yesterday'
  return new Date(startedAt).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
  })
}

/** "took 4 min" style length of a task, rounded for people rather than logs. */
export function tookLabel(durationMs: number): string {
  const minutes = Math.round(Math.max(0, durationMs) / 60_000)
  if (minutes < 1) return 'took under a minute'
  if (minutes < 60) return `took ${minutes} min`
  const hours = Math.floor(minutes / 60)
  const rest = minutes % 60
  return rest === 0 ? `took ${hours} h` : `took ${hours} h ${rest} min`
}

/** "Today at 2:41 PM", "Yesterday at 9:05 AM", then "Oct 21 at 4:10 PM". */
export function taskDayAndTime(startedAt: number, now: number): string {
  const days = daysBefore(startedAt, now)
  const day = days <= 0 ? 'Today' : taskWhen(startedAt, now)
  return `${day} at ${taskTime(startedAt)}`
}
