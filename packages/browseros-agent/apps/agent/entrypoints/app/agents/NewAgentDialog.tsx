import { AlertCircle, CheckCircle, Loader2, TriangleAlert } from 'lucide-react'
import type { FC } from 'react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import type {
  HarnessAdapterDescriptor,
  HarnessAgentAdapter,
} from './agent-harness-types'
import type { CreateAgentRuntime, ProviderOption } from './agents-page-types'
import { ProviderSelector } from './OpenClawControls'
import {
  type OpenClawCliProvider,
  type OpenClawCliProviderAuthStatus,
  OpenClawCliProviderStatusPanel,
} from './openclaw-cli-providers'

/** Probe result badge — extracted to reduce parent complexity */
const ProbeResultBadge: FC<{
  result: { healthy: boolean; error?: string } | null
}> = ({ result }) => {
  if (!result) return null
  if (result.healthy) {
    return (
      <span className="flex items-center gap-1 text-green-600 text-sm">
        <CheckCircle className="size-4" /> ACP ready
      </span>
    )
  }
  return (
    <span className="flex items-center gap-1 text-sm text-yellow-600">
      <TriangleAlert className="size-4" /> {result.error ?? 'Probe failed'}
    </span>
  )
}

interface NewAgentDialogProps {
  adapters: HarnessAdapterDescriptor[]
  canManageOpenClaw: boolean
  createError: string | null
  createRuntime: CreateAgentRuntime
  creating: boolean
  customCommand?: string
  customArgs?: string
  customLabel?: string
  customProbeResult?: { healthy: boolean; error?: string } | null
  customProbeLoading?: boolean
  defaultProviderId: string
  harnessAdapterId: HarnessAgentAdapter
  harnessModelId: string
  harnessReasoningEffort: string
  hermesProviders: ProviderOption[]
  hermesSelectedProviderId: string
  name: string
  open: boolean
  providers: ProviderOption[]
  selectedCliProvider: OpenClawCliProvider | undefined
  selectedProviderId: string
  cliAuthError: Error | null
  cliAuthLoading: boolean
  cliAuthStatus: OpenClawCliProviderAuthStatus | undefined
  onConnectCliProvider: () => void
  onCreate: () => void
  onOpenChange: (open: boolean) => void
  onRuntimeChange: (runtime: CreateAgentRuntime) => void
  onHarnessAdapterChange: (adapter: HarnessAgentAdapter) => void
  onHarnessModelChange: (modelId: string) => void
  onHarnessReasoningChange: (reasoningEffort: string) => void
  onHermesProviderChange: (providerId: string) => void
  onNameChange: (name: string) => void
  onProviderChange: (providerId: string) => void
  onCustomCommandChange?: (command: string) => void
  onCustomArgsChange?: (args: string) => void
  onCustomLabelChange?: (label: string) => void
  onProbeCustom?: () => void
  onImportAcpx?: () => void
}

export const NewAgentDialog: FC<NewAgentDialogProps> = ({
  adapters,
  canManageOpenClaw,
  createError,
  createRuntime,
  creating,
  customCommand = '',
  customArgs = '',
  customLabel = '',
  customProbeResult = null,
  customProbeLoading = false,
  defaultProviderId,
  harnessAdapterId,
  harnessModelId,
  harnessReasoningEffort,
  hermesProviders,
  hermesSelectedProviderId,
  name,
  open,
  providers,
  selectedCliProvider,
  selectedProviderId,
  cliAuthError,
  cliAuthLoading,
  cliAuthStatus,
  onConnectCliProvider,
  onCreate,
  onOpenChange,
  onRuntimeChange,
  onHarnessAdapterChange,
  onHarnessModelChange,
  onHarnessReasoningChange,
  onHermesProviderChange,
  onNameChange,
  onProviderChange,
  onCustomCommandChange = () => {},
  onCustomArgsChange = () => {},
  onCustomLabelChange = () => {},
  onProbeCustom = () => {},
  onImportAcpx = () => {},
}) => {
  const selectedHarnessAdapter =
    adapters.find((adapter) => adapter.id === harnessAdapterId) ?? adapters[0]
  const isHarnessRuntime = createRuntime !== 'openclaw'
  const isHermesRuntime = createRuntime === 'hermes'
  const isCustomRuntime = createRuntime === 'custom'
  const isClassicHarnessRuntime =
    isHarnessRuntime && !isHermesRuntime && !isCustomRuntime
  const openClawBlocked = createRuntime === 'openclaw' && !canManageOpenClaw
  const cliBlocked =
    createRuntime === 'openclaw' &&
    !!selectedCliProvider &&
    !cliAuthStatus?.loggedIn
  const hermesBlocked =
    isHermesRuntime &&
    (hermesProviders.length === 0 || !hermesSelectedProviderId)
  const customBlocked = isCustomRuntime && !customCommand.trim()
  const canCreate =
    Boolean(name.trim()) &&
    !creating &&
    !openClawBlocked &&
    !cliBlocked &&
    !hermesBlocked &&
    !customBlocked &&
    (createRuntime === 'openclaw'
      ? providers.length > 0
      : createRuntime === 'custom'
        ? customCommand.trim().length > 0
        : Boolean(selectedHarnessAdapter))

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>New Agent</DialogTitle>
        </DialogHeader>

        <div className="grid gap-4 py-2">
          {createError ? (
            <Alert variant="destructive">
              <AlertCircle className="size-4" />
              <AlertTitle>Create failed</AlertTitle>
              <AlertDescription>{createError}</AlertDescription>
            </Alert>
          ) : null}

          <div className="grid gap-2">
            <Label htmlFor="agent-name">Name</Label>
            <Input
              id="agent-name"
              value={name}
              onChange={(event) => onNameChange(event.target.value)}
              placeholder={
                createRuntime === 'openclaw' ? 'research-agent' : 'Review bot'
              }
              onKeyDown={(event) => {
                if (event.key === 'Enter' && canCreate) onCreate()
              }}
            />
          </div>

          <div className="grid gap-2">
            <Label htmlFor="agent-runtime">Adapter</Label>
            <Select
              value={createRuntime}
              onValueChange={(value) => {
                if (
                  value === 'openclaw' ||
                  value === 'claude' ||
                  value === 'codex' ||
                  value === 'hermes' ||
                  value === 'custom'
                ) {
                  onRuntimeChange(value)
                  if (value !== 'openclaw') onHarnessAdapterChange(value)
                }
              }}
            >
              <SelectTrigger id="agent-runtime">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {adapters.map((adapter) => (
                  <SelectItem key={adapter.id} value={adapter.id}>
                    {adapter.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {createRuntime === 'openclaw' ? (
            <>
              {openClawBlocked ? (
                <Alert>
                  <AlertCircle className="size-4" />
                  <AlertTitle>OpenClaw is not ready</AlertTitle>
                  <AlertDescription>
                    Start or set up the OpenClaw gateway before creating an
                    OpenClaw agent.
                  </AlertDescription>
                </Alert>
              ) : null}

              <ProviderSelector
                providers={providers}
                defaultProviderId={defaultProviderId}
                selectedId={selectedProviderId}
                onSelect={onProviderChange}
                hideApiKeyHint={!!selectedCliProvider}
              />

              {selectedCliProvider ? (
                <OpenClawCliProviderStatusPanel
                  provider={selectedCliProvider}
                  status={cliAuthStatus}
                  loading={cliAuthLoading}
                  fetchError={cliAuthError}
                  onConnect={onConnectCliProvider}
                />
              ) : null}
            </>
          ) : null}

          {isHermesRuntime ? (
            <ProviderSelector
              providers={hermesProviders}
              defaultProviderId={defaultProviderId}
              selectedId={hermesSelectedProviderId}
              onSelect={onHermesProviderChange}
            />
          ) : null}

          {isClassicHarnessRuntime ? (
            <>
              <div className="grid gap-2">
                <Label htmlFor="harness-model">Model</Label>
                <Select
                  value={harnessModelId}
                  onValueChange={onHarnessModelChange}
                >
                  <SelectTrigger id="harness-model">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {(selectedHarnessAdapter?.models ?? []).map((model) => (
                      <SelectItem key={model.id} value={model.id}>
                        {model.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="grid gap-2">
                <Label htmlFor="harness-effort">Reasoning</Label>
                <Select
                  value={harnessReasoningEffort}
                  onValueChange={onHarnessReasoningChange}
                >
                  <SelectTrigger id="harness-effort">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {(selectedHarnessAdapter?.reasoningEfforts ?? []).map(
                      (effort) => (
                        <SelectItem key={effort.id} value={effort.id}>
                          {effort.label}
                        </SelectItem>
                      ),
                    )}
                  </SelectContent>
                </Select>
              </div>
            </>
          ) : null}

          {isCustomRuntime ? (
            <>
              <div className="grid gap-2">
                <Label htmlFor="custom-command">Command</Label>
                <Input
                  id="custom-command"
                  value={customCommand}
                  onChange={(event) =>
                    onCustomCommandChange(event.target.value)
                  }
                  placeholder="e.g., gemini, ./bin/my-acp, npx opencode-ai acp"
                  onKeyDown={(event) => {
                    if (event.key === 'Enter' && canCreate) onCreate()
                  }}
                />
              </div>

              <div className="grid gap-2">
                <Label htmlFor="custom-args">Args</Label>
                <Input
                  id="custom-args"
                  value={customArgs}
                  onChange={(event) => onCustomArgsChange(event.target.value)}
                  placeholder="--acp, --profile ci"
                />
              </div>

              <div className="grid gap-2">
                <Label htmlFor="custom-label">Display Name</Label>
                <Input
                  id="custom-label"
                  value={customLabel}
                  onChange={(event) => onCustomLabelChange(event.target.value)}
                  placeholder="Optional label shown in the agent rail"
                />
              </div>

              <div className="flex items-center gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={onProbeCustom}
                  disabled={customProbeLoading || !customCommand.trim()}
                >
                  {customProbeLoading ? (
                    <Loader2 className="mr-1 size-3 animate-spin" />
                  ) : null}
                  TEST
                </Button>
                <ProbeResultBadge result={customProbeResult} />
              </div>

              <div className="flex items-center gap-2">
                <Button variant="outline" size="sm" onClick={onImportAcpx}>
                  Import from acpx
                </Button>
              </div>
            </>
          ) : null}
        </div>

        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={creating}
          >
            Cancel
          </Button>
          <Button disabled={!canCreate} onClick={onCreate}>
            {creating ? <Loader2 className="mr-2 size-4 animate-spin" /> : null}
            Create
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
