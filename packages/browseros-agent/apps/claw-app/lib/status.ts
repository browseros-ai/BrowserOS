import type {
  LiveSessionActivityState,
  SessionStatus,
} from '@browseros/claw-api'
import type { AnimatedBadgeStatus } from '@/components/motion/animated-badge'

/** Shared status labels and colors for cockpit and audit surfaces. */

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
  textClass: string
  bgClass: string
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

export function isActiveStatus(status: RunStatus): boolean {
  return (
    status === 'running' ||
    status === 'live' ||
    status === 'needs-ok' ||
    status === 'needs-human'
  )
}

export function isEndedStatus(status: RunStatus): boolean {
  return status === 'done' || status === 'stopped'
}

export interface SessionStatusBadge {
  label: string
  badge: AnimatedBadgeStatus
}

/** How a recorded task's status reads to a person, with its beUI badge tone. */
const SESSION_STATUS_BADGE: Record<SessionStatus, SessionStatusBadge> = {
  live: { label: 'Working', badge: 'loading' },
  done: { label: 'Done', badge: 'success' },
  failed: { label: 'Didn’t finish', badge: 'danger' },
  cancelled: { label: 'Stopped', badge: 'neutral' },
}

const IDLE_SESSION_BADGE: SessionStatusBadge = {
  label: 'Idle',
  badge: 'neutral',
}

/**
 * A task's badge, reading a live task as Idle once the server marks it inactive (no tool call
 * for a minute). Agents send no "finished" signal, so a session stays live until its MCP
 * connection closes or goes stale, which can be minutes after the agent is done.
 */
export function sessionStatusBadge(
  status: SessionStatus,
  liveState: LiveSessionActivityState | undefined,
): SessionStatusBadge {
  return status === 'live' && liveState === 'idle'
    ? IDLE_SESSION_BADGE
    : SESSION_STATUS_BADGE[status]
}
