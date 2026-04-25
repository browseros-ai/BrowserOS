import { useQuery } from '@tanstack/react-query'
import { CheckCircle2, Loader2, Terminal, TriangleAlert } from 'lucide-react'
import type { FC } from 'react'
import { Button } from '@/components/ui/button'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'

export interface OpenClawCliProvider {
  id: string
  displayName: string
  description: string
  models: readonly string[]
  authLoginCommand: string
}

export interface OpenClawCliProviderAuthStatus {
  installed: boolean
  loggedIn: boolean
  accountLabel?: string
  subscriptionLabel?: string
  error?: string
}

export interface OpenClawCliProviderOption {
  id: string
  type: string
  name: string
  modelId: string
}

const CLAUDE_CLI_PROVIDER: OpenClawCliProvider = {
  id: 'claude-cli',
  displayName: 'Anthropic Claude CLI',
  description: 'Uses your Claude.ai subscription via the Claude Code CLI',
  models: ['claude-sonnet-4-6', 'claude-opus-4-6', 'claude-haiku-4-5'],
  authLoginCommand: 'claude /login',
}

export const OPENCLAW_CLI_PROVIDERS: readonly OpenClawCliProvider[] = [
  CLAUDE_CLI_PROVIDER,
]

export function findOpenClawCliProviderById(
  id: string,
): OpenClawCliProvider | undefined {
  return OPENCLAW_CLI_PROVIDERS.find((provider) => provider.id === id)
}

export function buildOpenClawCliProviderOptions(): OpenClawCliProviderOption[] {
  return OPENCLAW_CLI_PROVIDERS.flatMap((provider) =>
    provider.models.map((modelId) => ({
      id: `${provider.id}/${modelId}`,
      type: provider.id,
      name: provider.displayName,
      modelId,
    })),
  )
}

async function fetchCliProviderAuthStatus(
  baseUrl: string,
  providerId: string,
): Promise<OpenClawCliProviderAuthStatus> {
  const res = await fetch(`${baseUrl}/claw/providers/${providerId}/auth-status`)
  if (!res.ok) {
    let message = `Auth status request failed (${res.status})`
    try {
      const body = (await res.json()) as { error?: string }
      if (body.error) message = body.error
    } catch {}
    throw new Error(message)
  }
  return res.json() as Promise<OpenClawCliProviderAuthStatus>
}

export function useOpenClawCliProviderAuthStatus(
  providerId: string,
  enabled: boolean,
) {
  const { baseUrl, isLoading: urlLoading } = useAgentServerUrl()
  return useQuery<OpenClawCliProviderAuthStatus, Error>({
    queryKey: ['openclaw-cli-auth', baseUrl, providerId],
    queryFn: () => fetchCliProviderAuthStatus(baseUrl as string, providerId),
    enabled: !!baseUrl && !urlLoading && enabled,
    refetchInterval: enabled ? 2000 : false,
  })
}

interface OpenClawCliProviderStatusPanelProps {
  provider: OpenClawCliProvider
  status: OpenClawCliProviderAuthStatus | undefined
  loading: boolean
  fetchError: Error | null
  onConnect: () => void
}

export const OpenClawCliProviderStatusPanel: FC<
  OpenClawCliProviderStatusPanelProps
> = ({ provider, status, loading, fetchError, onConnect }) => {
  // Initial fetch (no data yet).
  if (loading && !status) {
    return (
      <div className="flex items-center gap-2 rounded-md border border-border bg-muted/30 px-3 py-2 text-sm">
        <Loader2 className="size-4 animate-spin text-muted-foreground" />
        <span className="text-muted-foreground">
          Checking {provider.displayName} status…
        </span>
      </div>
    )
  }

  if (fetchError) {
    return (
      <div className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm">
        <TriangleAlert className="mt-0.5 size-4 text-destructive" />
        <div>
          <div className="font-medium text-destructive">
            Could not read {provider.displayName} status
          </div>
          <div className="text-muted-foreground text-xs">
            {fetchError.message}
          </div>
        </div>
      </div>
    )
  }

  if (!status) return null

  // Install failed or binary missing.
  if (!status.installed) {
    return (
      <div className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/5 px-3 py-2 text-sm">
        <TriangleAlert className="mt-0.5 size-4 text-amber-600" />
        <div>
          <div className="font-medium">
            {provider.displayName} not installed
          </div>
          <div className="text-muted-foreground text-xs">
            The gateway will try to install it on the next restart. If this
            persists, check your network and the gateway logs.
          </div>
        </div>
      </div>
    )
  }

  // Happy path.
  if (status.loggedIn) {
    const identityBits = [
      status.accountLabel,
      status.subscriptionLabel ? `(${status.subscriptionLabel})` : null,
    ].filter(Boolean)
    const identity = identityBits.length > 0 ? identityBits.join(' ') : 'Ready'
    return (
      <div className="flex items-center gap-2 rounded-md border border-emerald-500/40 bg-emerald-500/5 px-3 py-2 text-sm">
        <CheckCircle2 className="size-4 text-emerald-600" />
        <div className="min-w-0 flex-1">
          <div className="font-medium">Connected to {provider.displayName}</div>
          <div className="truncate text-muted-foreground text-xs">
            {identity}
          </div>
        </div>
      </div>
    )
  }

  // Installed but not logged in.
  return (
    <div className="flex flex-col gap-2 rounded-md border border-border bg-muted/30 px-3 py-3 text-sm">
      <div>
        <div className="font-medium">{provider.displayName} not set up</div>
        <div className="text-muted-foreground text-xs">
          {provider.description}
        </div>
        {status.error && (
          <div className="mt-1 text-destructive text-xs">{status.error}</div>
        )}
      </div>
      <Button size="sm" variant="outline" onClick={onConnect} className="w-fit">
        <Terminal className="mr-1 size-4" />
        Connect {provider.displayName}
      </Button>
    </div>
  )
}
