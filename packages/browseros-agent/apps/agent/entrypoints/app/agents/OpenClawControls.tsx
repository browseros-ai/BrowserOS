import {
  AlertCircle,
  Cpu,
  Loader2,
  Plus,
  RefreshCw,
  ShieldAlert,
  Square,
  TerminalSquare,
  WifiOff,
  Wrench,
} from 'lucide-react'
import type { FC } from 'react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import type { ProviderOption } from './agents-page-types'
import {
  CONTROL_PLANE_COPY,
  FALLBACK_CONTROL_PLANE_COPY,
} from './agents-page-types'
import type { getControlPlaneCopy } from './agents-page-utils'
import type { OpenClawStatus } from './useOpenClaw'

const StatusBadge: FC<{ status: OpenClawStatus['status'] }> = ({ status }) => {
  const variants: Record<
    OpenClawStatus['status'],
    {
      variant: 'default' | 'secondary' | 'outline' | 'destructive'
      label: string
    }
  > = {
    running: { variant: 'default', label: 'Running' },
    starting: { variant: 'secondary', label: 'Starting...' },
    stopped: { variant: 'outline', label: 'Stopped' },
    error: { variant: 'destructive', label: 'Error' },
    uninitialized: { variant: 'outline', label: 'Not Set Up' },
  }
  const current = variants[status] ?? {
    variant: 'outline' as const,
    label: 'Unknown',
  }
  return <Badge variant={current.variant}>{current.label}</Badge>
}

const ControlPlaneBadge: FC<{
  status: OpenClawStatus['controlPlaneStatus']
}> = ({ status }) => {
  const current = CONTROL_PLANE_COPY[status] ?? FALLBACK_CONTROL_PLANE_COPY
  return <Badge variant={current.badgeVariant}>{current.badgeLabel}</Badge>
}

interface ProviderSelectorProps {
  providers: ProviderOption[]
  defaultProviderId: string
  selectedId: string
  onSelect: (id: string) => void
  hideApiKeyHint?: boolean
}

export const ProviderSelector: FC<ProviderSelectorProps> = ({
  providers,
  defaultProviderId,
  selectedId,
  onSelect,
  hideApiKeyHint,
}) => {
  if (providers.length === 0) {
    return (
      <div className="space-y-2">
        <p className="font-medium text-sm">LLM Provider</p>
        <p className="text-muted-foreground text-sm">
          No compatible LLM providers configured.{' '}
          <a href="#/settings/ai" className="underline">
            Add one in AI settings
          </a>{' '}
          first.
        </p>
      </div>
    )
  }

  return (
    <div className="space-y-2">
      <Label htmlFor="provider-select">LLM Provider</Label>
      <Select value={selectedId} onValueChange={onSelect}>
        <SelectTrigger id="provider-select">
          <SelectValue placeholder="Select a provider" />
        </SelectTrigger>
        <SelectContent>
          {providers.map((provider) => (
            <SelectItem key={provider.id} value={provider.id}>
              {provider.name} - {provider.modelId}
              {provider.id === defaultProviderId ? ' (default)' : ''}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {!hideApiKeyHint && (
        <p className="text-muted-foreground text-xs">
          Uses your existing API key from BrowserOS settings. The key is passed
          to the container and never leaves your machine.
        </p>
      )}
    </div>
  )
}

interface AgentsPageHeaderProps {
  actionInProgress: boolean
  controlPlaneBusy: boolean
  reconnecting: boolean
  status: OpenClawStatus | null
  onCreateAgent: () => void
  onOpenTerminal: () => void
  onReconnect: () => void
  onRefresh: () => void
  onRestart: () => void
  onStop: () => void
}

export const AgentsPageHeader: FC<AgentsPageHeaderProps> = ({
  actionInProgress,
  controlPlaneBusy,
  reconnecting,
  status,
  onCreateAgent,
  onOpenTerminal,
  onReconnect,
  onRefresh,
  onRestart,
  onStop,
}) => (
  <div className="flex flex-wrap items-center justify-between gap-3">
    <div>
      <h1 className="font-semibold text-2xl tracking-normal">Agents</h1>
      <p className="text-muted-foreground text-sm">
        OpenClaw, Claude Code, and Codex agents
      </p>
    </div>

    <div className="flex flex-wrap items-center gap-2">
      {status ? (
        <>
          <StatusBadge status={status.status} />
          {status.status !== 'uninitialized' && (
            <ControlPlaneBadge status={status.controlPlaneStatus} />
          )}
        </>
      ) : null}

      {status?.status === 'running' &&
      status.controlPlaneStatus !== 'connected' ? (
        <Button
          variant="outline"
          onClick={onReconnect}
          disabled={actionInProgress || controlPlaneBusy}
        >
          {reconnecting ? (
            <Loader2 className="mr-2 size-4 animate-spin" />
          ) : (
            <RefreshCw className="mr-2 size-4" />
          )}
          Retry Connection
        </Button>
      ) : null}

      {status?.status === 'running' ? (
        <>
          <Button
            variant="ghost"
            size="icon"
            onClick={onRestart}
            disabled={actionInProgress}
            title="Restart gateway"
          >
            <RefreshCw className="size-4" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            onClick={onStop}
            disabled={actionInProgress}
            title="Stop gateway"
          >
            <Square className="size-4" />
          </Button>
          <Button variant="outline" onClick={onOpenTerminal}>
            <TerminalSquare className="mr-2 size-4" />
            Terminal
          </Button>
        </>
      ) : null}

      <Button variant="ghost" size="icon" onClick={onRefresh} title="Refresh">
        <RefreshCw className="size-4" />
      </Button>
      <Button onClick={onCreateAgent}>
        <Plus className="mr-2 size-4" />
        New Agent
      </Button>
    </div>
  </div>
)

export function LifecycleAlert({ message }: { message: string }) {
  return (
    <Alert>
      <Loader2 className="size-4 animate-spin" />
      <AlertTitle>{message}</AlertTitle>
    </Alert>
  )
}

export function InlineErrorAlert({
  message,
  onDismiss,
}: {
  message: string
  onDismiss: () => void
}) {
  return (
    <Alert variant="destructive">
      <AlertCircle className="size-4" />
      <AlertTitle>Agent action failed</AlertTitle>
      <AlertDescription>
        <p>{message}</p>
        <div className="mt-2">
          <Button variant="outline" size="sm" onClick={onDismiss}>
            Dismiss
          </Button>
        </div>
      </AlertDescription>
    </Alert>
  )
}

interface ControlPlaneAlertProps {
  actionInProgress: boolean
  controlPlaneBusy: boolean
  controlPlaneCopy: ReturnType<typeof getControlPlaneCopy>
  reconnecting: boolean
  recoveryDetail: string | null
  status: OpenClawStatus
  onReconnect: () => void
  onRestart: () => void
}

export const ControlPlaneAlert: FC<ControlPlaneAlertProps> = ({
  actionInProgress,
  controlPlaneBusy,
  controlPlaneCopy,
  reconnecting,
  recoveryDetail,
  status,
  onReconnect,
  onRestart,
}) => (
  <Alert
    variant={status.controlPlaneStatus === 'failed' ? 'destructive' : 'default'}
  >
    {status.controlPlaneStatus === 'failed' ? (
      <ShieldAlert className="size-4" />
    ) : status.controlPlaneStatus === 'recovering' ? (
      <Wrench className="size-4" />
    ) : (
      <WifiOff className="size-4" />
    )}
    <AlertTitle>{controlPlaneCopy.title}</AlertTitle>
    <AlertDescription>
      <p>{controlPlaneCopy.description}</p>
      {recoveryDetail ? <p>{recoveryDetail}</p> : null}
      <div className="mt-2 flex flex-wrap gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={onReconnect}
          disabled={actionInProgress || controlPlaneBusy}
        >
          {reconnecting ? (
            <Loader2 className="mr-2 size-4 animate-spin" />
          ) : (
            <RefreshCw className="mr-2 size-4" />
          )}
          Retry Connection
        </Button>
        <Button
          variant="outline"
          size="sm"
          onClick={onRestart}
          disabled={actionInProgress}
        >
          Restart Gateway
        </Button>
      </div>
    </AlertDescription>
  </Alert>
)

interface GatewayStateCardsProps {
  actionInProgress: boolean
  status: OpenClawStatus | null
  onOpenSetup: () => void
  onRestart: () => void
  onStart: () => void
}

export const GatewayStateCards: FC<GatewayStateCardsProps> = ({
  actionInProgress,
  status,
  onOpenSetup,
  onRestart,
  onStart,
}) => (
  <>
    {status?.status === 'uninitialized' ? (
      <Card>
        <CardContent className="flex flex-col items-center gap-4 py-12">
          <Cpu className="size-12 text-muted-foreground" />
          <div className="text-center">
            <h3 className="font-semibold text-lg">Set Up OpenClaw</h3>
            <p className="text-muted-foreground text-sm">
              {status.podmanAvailable
                ? 'Create a local BrowserOS VM to run autonomous agents with full tool access.'
                : 'BrowserOS VM runtime is unavailable on this system.'}
            </p>
          </div>
          {status.podmanAvailable ? (
            <Button onClick={onOpenSetup}>Set Up Now</Button>
          ) : null}
        </CardContent>
      </Card>
    ) : null}

    {status?.status === 'stopped' ? (
      <Card>
        <CardContent className="flex flex-col items-center gap-4 py-12">
          <Cpu className="size-12 text-muted-foreground" />
          <div className="text-center">
            <h3 className="font-semibold text-lg">Gateway Stopped</h3>
            <p className="text-muted-foreground text-sm">
              The OpenClaw gateway is not running.
            </p>
          </div>
          <Button onClick={onStart} disabled={actionInProgress}>
            Start Gateway
          </Button>
        </CardContent>
      </Card>
    ) : null}

    {status?.status === 'error' ? (
      <Card className="border-destructive">
        <CardContent className="flex flex-col items-center gap-4 py-12">
          <AlertCircle className="size-12 text-destructive" />
          <div className="text-center">
            <h3 className="font-semibold text-lg">Gateway Error</h3>
            <p className="text-muted-foreground text-sm">
              {status.error ?? status.lastGatewayError}
            </p>
          </div>
          <div className="flex gap-2">
            <Button onClick={onStart} disabled={actionInProgress}>
              Start Gateway
            </Button>
            <Button
              variant="outline"
              onClick={onRestart}
              disabled={actionInProgress}
            >
              Restart Gateway
            </Button>
          </div>
        </CardContent>
      </Card>
    ) : null}
  </>
)
