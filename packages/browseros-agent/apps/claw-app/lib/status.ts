import type { LiveSessionState, SessionStatus } from '@browseros/claw-api'
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

const WAITING_SESSION_BADGE: SessionStatusBadge = {
  label: 'Waiting for you',
  badge: 'warning',
}

/**
 * The live snapshot carries pending help even when tool activity has gone idle.
 * Recorded terminal outcomes win over a stale live snapshot after a session ends.
 */
export function sessionStatusBadge(
  status: SessionStatus,
  liveState: LiveSessionState | undefined,
): SessionStatusBadge {
  if (status !== 'live') return SESSION_STATUS_BADGE[status]
  if (liveState?.helpRequest) return WAITING_SESSION_BADGE
  return liveState?.state === 'idle'
    ? IDLE_SESSION_BADGE
    : SESSION_STATUS_BADGE.live
}
