import { RefreshCw, Terminal } from 'lucide-react'
import type { FC } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import { cn } from '@/lib/utils'
import type { OpenClawStatus } from './useOpenClaw'

interface GatewayStatusBarProps {
  status: OpenClawStatus | null
  onOpenTerminal: () => void
  onRefresh: () => void
}

/**
 * Compact one-line status bar for the OpenClaw gateway. Lives between
 * the page header and the agent list when at least one OpenClaw agent
 * is in the picture; collapses to nothing when the user only has
 * Claude/Codex agents.
 *
 * Only renders the lifecycle pills + Terminal/Refresh affordances —
 * the existing `GatewayStateCards` (start, restart, setup) keep their
 * own slot below the list because they're action-heavy. This bar is
 * for at-a-glance reassurance: "yes, the gateway is running and the
 * control plane is connected."
 */
export const GatewayStatusBar: FC<GatewayStatusBarProps> = ({
  status,
  onOpenTerminal,
  onRefresh,
}) => {
  if (!status) return null

  const runningPill = pillForRuntimeStatus(status.status)
  const controlPlanePill = pillForControlPlane(status.controlPlaneStatus)

  return (
    <div className="rounded-xl border border-border bg-card px-4 py-3 shadow-sm">
      <div className="flex items-center gap-3 text-sm">
        <span className="font-medium text-muted-foreground">
          OpenClaw gateway
        </span>
        <Badge
          variant={runningPill.variant}
          className={cn('gap-1.5', runningPill.className)}
        >
          <span
            className={cn(
              'inline-block h-1.5 w-1.5 rounded-full',
              runningPill.dot,
            )}
          />
          {runningPill.label}
        </Badge>
        <Badge
          variant={controlPlanePill.variant}
          className={cn('gap-1.5', controlPlanePill.className)}
        >
          <span
            className={cn(
              'inline-block h-1.5 w-1.5 rounded-full',
              controlPlanePill.dot,
            )}
          />
          {controlPlanePill.label}
        </Badge>
        <Separator orientation="vertical" className="h-4" />
        <Button variant="ghost" size="sm" onClick={onOpenTerminal}>
          <Terminal className="mr-1.5 h-3.5 w-3.5" />
          Terminal
        </Button>
        <Button
          variant="ghost"
          size="icon"
          onClick={onRefresh}
          aria-label="Refresh gateway status"
          className="ml-auto h-8 w-8"
        >
          <RefreshCw className="h-4 w-4" />
        </Button>
      </div>
    </div>
  )
}

type PillKind = {
  variant: 'default' | 'secondary' | 'outline' | 'destructive'
  label: string
  dot: string
  className?: string
}

function pillForRuntimeStatus(status: OpenClawStatus['status']): PillKind {
  switch (status) {
    case 'running':
      return {
        variant: 'secondary',
        label: 'Running',
        dot: 'bg-emerald-500',
        className: 'bg-emerald-50 text-emerald-900 hover:bg-emerald-50',
      }
    case 'starting':
      return {
        variant: 'secondary',
        label: 'Starting',
        dot: 'bg-amber-500 animate-pulse',
        className: 'bg-amber-50 text-amber-900 hover:bg-amber-50',
      }
    case 'stopped':
      return {
        variant: 'outline',
        label: 'Stopped',
        dot: 'bg-muted-foreground/40',
      }
    case 'error':
      return {
        variant: 'destructive',
        label: 'Error',
        dot: 'bg-destructive-foreground',
      }
    default:
      return {
        variant: 'outline',
        label: 'Unknown',
        dot: 'bg-muted-foreground/40',
      }
  }
}

function pillForControlPlane(
  status: OpenClawStatus['controlPlaneStatus'],
): PillKind {
  switch (status) {
    case 'connected':
      return {
        variant: 'secondary',
        label: 'Control plane connected',
        dot: 'bg-emerald-500',
        className: 'bg-emerald-50 text-emerald-900 hover:bg-emerald-50',
      }
    case 'connecting':
      return {
        variant: 'secondary',
        label: 'Connecting',
        dot: 'bg-amber-500 animate-pulse',
        className: 'bg-amber-50 text-amber-900 hover:bg-amber-50',
      }
    case 'reconnecting':
      return {
        variant: 'secondary',
        label: 'Reconnecting',
        dot: 'bg-amber-500 animate-pulse',
        className: 'bg-amber-50 text-amber-900 hover:bg-amber-50',
      }
    case 'recovering':
      return {
        variant: 'secondary',
        label: 'Recovering',
        dot: 'bg-amber-500 animate-pulse',
        className: 'bg-amber-50 text-amber-900 hover:bg-amber-50',
      }
    case 'failed':
      return {
        variant: 'destructive',
        label: 'Needs attention',
        dot: 'bg-destructive-foreground',
      }
    default:
      return {
        variant: 'outline',
        label: 'Disconnected',
        dot: 'bg-muted-foreground/40',
      }
  }
}
