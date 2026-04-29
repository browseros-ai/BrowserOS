import { Loader2, RotateCcw, Terminal } from 'lucide-react'
import type { FC, ReactNode } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import type { OpenClawStatus } from './useOpenClaw'

interface GatewayStatusBarProps {
  status: OpenClawStatus | null
  /** Disabled while a gateway lifecycle mutation is mid-flight. */
  actionInProgress: boolean
  onOpenTerminal: () => void
  onRestart: () => void
}

/**
 * Compact one-line status bar for the OpenClaw gateway. Renders the
 * lifecycle pills (Running / Control plane connected) plus a Terminal
 * escape hatch and a Restart Gateway action. Lives between the page
 * header and the agent list when at least one OpenClaw agent is in
 * the merged list; collapses to nothing for Claude/Codex-only setups.
 *
 * Status is sourced from `GET /agents`'s `gateway` field — the agents
 * page no longer polls `/claw/status` directly. One endpoint, one
 * 5s interval, no duplicate state.
 */
export const GatewayStatusBar: FC<GatewayStatusBarProps> = ({
  status,
  actionInProgress,
  onOpenTerminal,
  onRestart,
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
        <WithTooltip label="Open a shell into the OpenClaw gateway container for raw CLI access (config edits, session inspection).">
          <Button variant="ghost" size="sm" onClick={onOpenTerminal}>
            <Terminal className="mr-1.5 h-3.5 w-3.5" />
            Terminal
          </Button>
        </WithTooltip>
        <WithTooltip label="Restart the OpenClaw gateway. Useful when the gateway is stuck or after editing provider config.">
          <Button
            variant="ghost"
            size="sm"
            onClick={onRestart}
            disabled={actionInProgress}
            className="ml-auto"
          >
            {actionInProgress ? (
              <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
            ) : (
              <RotateCcw className="mr-1.5 h-3.5 w-3.5" />
            )}
            Restart Gateway
          </Button>
        </WithTooltip>
      </div>
    </div>
  )
}

const WithTooltip: FC<{ label: string; children: ReactNode }> = ({
  label,
  children,
}) => (
  <TooltipProvider delayDuration={250}>
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent side="bottom" className="max-w-xs text-xs">
        {label}
      </TooltipContent>
    </Tooltip>
  </TooltipProvider>
)

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
