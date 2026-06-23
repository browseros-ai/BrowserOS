import {
  AlertCircle,
  CheckCircle2,
  Clock,
  Lock,
  ShieldCheck,
  User,
  X,
} from 'lucide-react'
import type { ComponentType } from 'react'
import type { RunStatus } from '@/lib/status'
import type { RunHarness, RunRow } from '@/modules/api/runs.hooks'

export type AuditFilter = 'all' | 'running' | 'blocked' | 'done'

export const AUDIT_FILTERS: readonly { key: AuditFilter; label: string }[] = [
  { key: 'all', label: 'All' },
  { key: 'running', label: 'Running' },
  { key: 'blocked', label: 'Blocked' },
  { key: 'done', label: 'Completed' },
]

export interface AuditIcon {
  Icon: ComponentType<{ className?: string }>
  className: string
}

const ICON_MAP: Record<RunStatus, AuditIcon> = {
  running: { Icon: AlertCircle, className: 'text-accent' },
  live: { Icon: AlertCircle, className: 'text-accent' },
  'needs-ok': { Icon: ShieldCheck, className: 'text-amber' },
  'needs-human': { Icon: User, className: 'text-amber' },
  blocked: { Icon: Lock, className: 'text-red' },
  done: { Icon: CheckCircle2, className: 'text-green' },
  stopped: { Icon: X, className: 'text-ink-3' },
  asleep: { Icon: Clock, className: 'text-ink-3' },
  idle: { Icon: Clock, className: 'text-ink-3' },
  stuck: { Icon: AlertCircle, className: 'text-amber' },
}

export function auditIconFor(status: RunStatus): AuditIcon {
  return ICON_MAP[status] ?? { Icon: Clock, className: 'text-ink-3' }
}

export function filterRuns(
  runs: readonly RunRow[],
  filter: AuditFilter,
): RunRow[] {
  if (filter === 'all') return [...runs]
  if (filter === 'running') {
    return runs.filter(
      (r) =>
        r.status === 'running' ||
        r.status === 'live' ||
        r.status === 'needs-ok',
    )
  }
  return runs.filter((r) => r.status === filter)
}

export function countLiveRuns(runs: readonly RunRow[]): number {
  return runs.filter((r) => r.status === 'running' || r.status === 'live')
    .length
}

export function isCheckmarkHarness(harness: RunHarness): boolean {
  return harness !== 'Codex'
}
