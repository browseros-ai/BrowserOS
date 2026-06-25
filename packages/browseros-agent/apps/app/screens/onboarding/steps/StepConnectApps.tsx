import { Check, Loader2, Plug } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { McpServerIcon } from '@/components/mcp/McpServerIcon'
import { Button } from '@/components/ui/button'
import {
  ONBOARDING_APP_CONNECTED_EVENT,
  ONBOARDING_CONNECT_APPS_SKIPPED_EVENT,
  ONBOARDING_CONNECT_APPS_VIEWED_EVENT,
  ONBOARDING_STEP_COMPLETED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { useMcpServers } from '@/lib/mcp/mcpServerStorage'
import { track } from '@/lib/metrics/track'
import { sentry } from '@/lib/sentry/sentry'
import { useAddManagedServer } from '@/modules/mcp/add-managed-server.hooks'
import { useGetUserMCPIntegrations } from '@/modules/mcp/user-integrations.hooks'
import { type StepDirection, StepTransition } from './StepTransition'

export interface StepConnectAppsProps {
  direction: StepDirection
  onContinue: () => void
}

const RECOMMENDED_APPS = [
  { name: 'Gmail', description: 'Read and send emails' },
  { name: 'Google Calendar', description: 'View and manage events' },
]

const MORE_APPS = [
  { name: 'Notion' },
  { name: 'Slack' },
  { name: 'GitHub' },
  { name: 'Linear' },
  { name: 'Jira' },
  { name: 'Google Docs' },
]

export const StepConnectApps = ({
  direction,
  onContinue,
}: StepConnectAppsProps) => {
  const { servers, addServer } = useMcpServers()
  const { trigger: addManagedServerMutation } = useAddManagedServer()
  const {
    data: userIntegrations,
    isLoading: isIntegrationsLoading,
    mutate: refreshIntegrations,
  } = useGetUserMCPIntegrations()
  const [connectingApp, setConnectingApp] = useState<string | null>(null)
  const [isPolling, setIsPolling] = useState(false)
  const pollingRef = useRef<ReturnType<typeof setInterval> | null>(null)

  useEffect(() => {
    track(ONBOARDING_CONNECT_APPS_VIEWED_EVENT)
  }, [])

  useEffect(() => {
    if (!isPolling) return

    pollingRef.current = setInterval(() => {
      refreshIntegrations()
    }, 3000)

    return () => {
      if (pollingRef.current) {
        clearInterval(pollingRef.current)
        pollingRef.current = null
      }
    }
  }, [isPolling, refreshIntegrations])

  const isAppConnected = (appName: string) => {
    return userIntegrations?.integrations?.some(
      (i) => i.name === appName && i.is_authenticated,
    )
  }

  // biome-ignore lint/correctness/useExhaustiveDependencies: Stop polling when both recommended apps are connected
  useEffect(() => {
    const allRecommendedConnected = RECOMMENDED_APPS.every((app) =>
      isAppConnected(app.name),
    )
    if (allRecommendedConnected && isPolling) {
      setIsPolling(false)
    }
  }, [userIntegrations, isPolling])

  const handleConnect = async (appName: string) => {
    setConnectingApp(appName)
    try {
      const response = await addManagedServerMutation({ serverName: appName })

      const alreadyAdded = servers?.some((s) => s.managedServerName === appName)
      if (!alreadyAdded) {
        addServer({
          id: Date.now().toString(),
          displayName: appName,
          type: 'managed',
          managedServerName: appName,
          managedServerDescription:
            RECOMMENDED_APPS.find((a) => a.name === appName)?.description ?? '',
        })
      }

      if (response.oauthUrl) {
        setIsPolling(true)
        window.open(response.oauthUrl, '_blank')?.focus()
      }

      track(ONBOARDING_APP_CONNECTED_EVENT, { app_name: appName })
    } catch (e) {
      sentry.captureException(e, {
        extra: { message: 'Failed to connect app during onboarding', appName },
      })
    } finally {
      setConnectingApp(null)
    }
  }

  const handleContinue = () => {
    track(ONBOARDING_STEP_COMPLETED_EVENT, {
      step: 2,
      step_name: 'connect_apps',
      gmail_connected: !!isAppConnected('Gmail'),
      calendar_connected: !!isAppConnected('Google Calendar'),
    })
    onContinue()
  }

  const handleSkip = () => {
    track(ONBOARDING_CONNECT_APPS_SKIPPED_EVENT)
    track(ONBOARDING_STEP_COMPLETED_EVENT, {
      step: 2,
      step_name: 'connect_apps',
      skipped: true,
    })
    onContinue()
  }

  return (
    <StepTransition direction={direction}>
      <div className="flex h-full flex-col items-center justify-center">
        <div className="w-full max-w-md space-y-6">
          <div className="space-y-2 text-center">
            <div className="mx-auto mb-4 flex size-12 items-center justify-center rounded-full bg-[var(--accent-orange)]/10">
              <Plug className="size-6 text-[var(--accent-orange)]" />
            </div>
            <h2 className="font-bold text-3xl tracking-tight">
              Connect your apps
            </h2>
            <p className="text-base text-muted-foreground">
              Let your assistant work with your email, calendar, and more
            </p>
          </div>

          {/* Recommended apps */}
          <div className="space-y-2">
            <div className="font-medium text-muted-foreground text-xs uppercase tracking-wider">
              Recommended
            </div>
            <div className="space-y-2">
              {RECOMMENDED_APPS.map((app) => {
                const connected = isAppConnected(app.name)
                const isConnecting = connectingApp === app.name

                return (
                  <div
                    key={app.name}
                    className="flex items-center gap-4 rounded-lg border border-border bg-card p-4 transition-all hover:border-[var(--accent-orange)]/50"
                  >
                    <div className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-muted">
                      <McpServerIcon serverName={app.name} size={24} />
                    </div>
                    <div className="flex-1">
                      <div className="font-medium text-sm">{app.name}</div>
                      <div className="text-muted-foreground text-xs">
                        {app.description}
                      </div>
                    </div>
                    {isIntegrationsLoading ? (
                      <Loader2 className="size-4 animate-spin text-muted-foreground" />
                    ) : connected ? (
                      <span className="flex items-center gap-1 rounded-full bg-green-500/10 px-2 py-1 font-medium text-green-600 text-xs">
                        <Check className="size-3" />
                        Connected
                      </span>
                    ) : (
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => handleConnect(app.name)}
                        disabled={isConnecting}
                      >
                        {isConnecting ? (
                          <Loader2 className="size-3 animate-spin" />
                        ) : (
                          'Connect'
                        )}
                      </Button>
                    )}
                  </div>
                )
              })}
            </div>
          </div>

          {/* More apps - compact icon grid */}
          <div className="space-y-2">
            <div className="font-medium text-muted-foreground text-xs uppercase tracking-wider">
              More apps
            </div>
            <div className="grid grid-cols-3 gap-2">
              {MORE_APPS.map((app) => {
                const connected = isAppConnected(app.name)
                const isConnecting = connectingApp === app.name

                return (
                  <button
                    type="button"
                    key={app.name}
                    onClick={() => !connected && handleConnect(app.name)}
                    disabled={isConnecting || connected}
                    className="flex flex-col items-center gap-1.5 rounded-lg border border-border bg-card p-3 transition-all hover:border-[var(--accent-orange)]/50 disabled:opacity-60"
                  >
                    <div className="relative">
                      <McpServerIcon serverName={app.name} size={24} />
                      {connected && (
                        <Check className="absolute -top-1 -right-1 size-3 rounded-full bg-green-500 p-0.5 text-white" />
                      )}
                    </div>
                    <span className="text-[11px] text-muted-foreground">
                      {app.name}
                    </span>
                  </button>
                )
              })}
            </div>
            <p className="text-center text-muted-foreground text-xs">
              40+ apps available in Settings
            </p>
          </div>

          <Button
            onClick={handleContinue}
            className="w-full bg-[var(--accent-orange)] text-white hover:bg-[var(--accent-orange)]/90"
          >
            Continue
          </Button>

          <div className="text-center">
            <Button
              variant="ghost"
              onClick={handleSkip}
              className="text-muted-foreground"
            >
              Skip for now
            </Button>
          </div>
        </div>
      </div>
    </StepTransition>
  )
}
