/**
 * Central RunStatus union + display metadata.
 *
 * The cockpit, governance audit, and recent-activity rows all map the
 * same underlying status to consistent label + color + pulse-dot
 * behaviour. Splitting that map out here keeps the StatusBadge a thin
 * wrapper and prevents the colors drifting between the dashboard and
 * the audit trail.
 */

export type RunStatus =
  | 'running'
  | 'live'
  | 'needs-ok'
  | 'needs-human'
  | 'blocked'
  | 'stopped'
  | 'done'
  | 'asleep'
  | 'idle'
  | 'stuck'

export interface StatusMeta {
  label: string
  /** Tailwind utility for the text color (token-keyed) */
  textClass: string
  /** Tailwind utility for the background tint */
  bgClass: string
  /** Whether the badge animates with a pulse-dot leading the label */
  pulse: boolean
}

export const STATUS_META: Record<RunStatus, StatusMeta> = {
  running: {
    label: 'Running',
    textClass: 'text-green',
    bgClass: 'bg-green-tint',
    pulse: true,
  },
  live: {
    label: 'Live',
    textClass: 'text-green',
    bgClass: 'bg-green-tint',
    pulse: true,
  },
  'needs-ok': {
    label: 'Needs OK',
    textClass: 'text-amber',
    bgClass: 'bg-amber-tint',
    pulse: false,
  },
  'needs-human': {
    label: 'Your turn',
    textClass: 'text-amber',
    bgClass: 'bg-amber-tint',
    pulse: false,
  },
  blocked: {
    label: 'Blocked',
    textClass: 'text-red',
    bgClass: 'bg-red-tint',
    pulse: false,
  },
  stopped: {
    label: 'Stopped',
    textClass: 'text-ink-3',
    bgClass: 'bg-bg-sunken',
    pulse: false,
  },
  done: {
    label: 'Completed',
    textClass: 'text-green',
    bgClass: 'bg-green-tint',
    pulse: false,
  },
  asleep: {
    label: 'Asleep',
    textClass: 'text-ink-3',
    bgClass: 'bg-bg-sunken',
    pulse: false,
  },
  idle: {
    label: 'Idle',
    textClass: 'text-ink-3',
    bgClass: 'bg-bg-sunken',
    pulse: false,
  },
  stuck: {
    label: 'Stuck',
    textClass: 'text-amber',
    bgClass: 'bg-amber-tint',
    pulse: false,
  },
}

/** Convenience: status that represents an in-flight run. */
export function isActiveStatus(status: RunStatus): boolean {
  return (
    status === 'running' ||
    status === 'live' ||
    status === 'needs-ok' ||
    status === 'needs-human'
  )
}

/** Convenience: terminal statuses where the run no longer changes. */
export function isEndedStatus(status: RunStatus): boolean {
  return status === 'done' || status === 'stopped'
}
