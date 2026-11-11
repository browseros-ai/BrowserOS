/** Display formats shared by the page's components. Tokens always carry "≈" (they are estimates). */

import type { TaskIdentification, TextSize } from '../shared/api'

const numbers = new Intl.NumberFormat('en-US')

export function tokens(size: TextSize | null): string {
  return size ? `≈${numbers.format(size.tokens)} tok` : '–'
}

export function duration(ms: number | null): string {
  if (ms === null) return '–'
  if (ms < 1000) return `${ms} ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`
  const minutes = Math.floor(ms / 60_000)
  return `${minutes} min ${Math.round((ms % 60_000) / 1000)} s`
}

export function clockTime(iso: string): string {
  return new Date(iso).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

/** How the viewer grouped this task, in words. */
export function identification(by: TaskIdentification): string {
  switch (by.kind) {
    case 'agent-task-id':
      return `the agent's task id ${by.taskId}`
    case 'process':
      return `process ${by.pid}${by.command ? ` (${by.command})` : ''}`
    case 'client-idle':
      return `client name, new task after ${by.idleMinutes} min idle (${by.reason})`
  }
}

/** Pretty JSON when the text is JSON, the text otherwise. */
export function prettyJson(value: unknown): string {
  return JSON.stringify(value, null, 2) ?? String(value)
}
