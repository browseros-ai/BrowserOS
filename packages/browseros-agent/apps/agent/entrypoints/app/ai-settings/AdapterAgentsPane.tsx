import { Plus, Settings2 } from 'lucide-react'
import { type FC, useEffect, useMemo, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { AdapterIcon, adapterLabel } from '@/entrypoints/app/agents/AdapterIcon'
import { AgentList } from '@/entrypoints/app/agents/AgentList'
import {
  adapterHealthLabel,
  adapterHealthMeta,
  adapterHealthTone,
} from '@/entrypoints/app/agents/adapter-health'
import type {
  HarnessAdapterHealth,
  HarnessAgent,
  HarnessAgentAdapter,
} from '@/entrypoints/app/agents/agent-harness-types'
import { useDefaultAgentName } from '@/entrypoints/app/agents/agents-page-hooks'
import type { AgentListItem } from '@/entrypoints/app/agents/agents-page-types'
import { toHarnessListItem } from '@/entrypoints/app/agents/agents-page-utils'
import { NewAgentDialog } from '@/entrypoints/app/agents/NewAgentDialog'
import { InlineErrorAlert } from '@/entrypoints/app/agents/PageAlerts'
import {
  useAgentAdapters,
  useCreateHarnessAgent,
  useDeleteHarnessAgent,
  useHarnessAgents,
  useUpdateHarnessAgent,
} from '@/entrypoints/app/agents/useAgents'
import {
  AGENT_CREATED_EVENT,
  AGENT_DELETED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import { cn } from '@/lib/utils'

interface AdapterAgentsPaneProps {
  adapterId: HarnessAgentAdapter
}

/**
 * Settings detail pane for a single harness adapter (Claude / Codex). Shows the
 * adapter's runtime health + defaults and owns CRUD for that adapter's named
 * agent instances. The create dialog is locked to this adapter (its only option),
 * so Hermes is never reachable here.
 */
export const AdapterAgentsPane: FC<AdapterAgentsPaneProps> = ({
  adapterId,
}) => {
  const { adapters } = useAgentAdapters()
  const { harnessAgents, loading } = useHarnessAgents()
  const createHarnessAgent = useCreateHarnessAgent()
  const deleteHarnessAgent = useDeleteHarnessAgent()
  const updateHarnessAgent = useUpdateHarnessAgent()

  const adapter = adapters.find((entry) => entry.id === adapterId)
  const label = adapter?.name || adapterLabel(adapterId)

  const adapterAgents = useMemo(
    () => harnessAgents.filter((agent) => agent.adapter === adapterId),
    [harnessAgents, adapterId],
  )

  const [createOpen, setCreateOpen] = useState(false)
  const [newName, setNewName] = useState('')
  const [modelId, setModelId] = useState('')
  const [reasoningEffort, setReasoningEffort] = useState('')
  const [createError, setCreateError] = useState<string | null>(null)
  const [pageError, setPageError] = useState<string | null>(null)
  const [deletingAgentKey, setDeletingAgentKey] = useState<string | null>(null)
  const [executablePath, setExecutablePath] = useState('')
  const [isSavingPath, setIsSavingPath] = useState(false)

  // Load executable path from storage on mount
  useEffect(() => {
    const key = `local:${adapterId}:executablePath`
    storage.getItem<string>(key).then((val) => {
      if (val) setExecutablePath(val)
    })
  }, [adapterId])

  const saveExecutablePath = async () => {
    setIsSavingPath(true)
    try {
      const key = `local:${adapterId}:executablePath`
      await storage.setItem(key, executablePath)
      toast.success('Executable path updated. Restart the server to apply.')
    } catch (err) {
      toast.error('Failed to save executable path')
    } finally {
      setIsSavingPath(false)
    }
  }

  useDefaultAgentName(createOpen, setNewName)
  // Seed model/reasoning from the adapter defaults whenever the dialog opens.
  useEffect(() => {
    if (!createOpen || !adapter) return
    setModelId((current) => current || adapter.defaultModelId)
    setReasoningEffort((current) => current || adapter.defaultReasoningEffort)
  }, [createOpen, adapter])

  const listItems = useMemo<AgentListItem[]>(
    () => adapterAgents.map(toHarnessListItem),
    [adapterAgents],
  )
  const harnessAgentLookup = useMemo(() => {
    const map = new Map<string, HarnessAgent>()
    for (const agent of adapterAgents) map.set(agent.id, agent)
    return map
  }, [adapterAgents])
  const activity = useMemo(() => {
    const map: Record<
      string,
      {
        status: 'working' | 'idle' | 'asleep' | 'error'
        lastUsedAt: number | null
      }
    > = {}
    for (const agent of adapterAgents) {
      if (!agent.status) continue
      map[agent.id] = {
        status: agent.status,
        lastUsedAt: agent.lastUsedAt ?? null,
      }
    }
    return map
  }, [adapterAgents])

  const handleCreate = async () => {
    if (!newName.trim()) return
    setCreateError(null)
    try {
      await createHarnessAgent.mutateAsync({
        name: newName.trim(),
        adapter: adapterId,
        modelId: modelId || undefined,
        reasoningEffort: reasoningEffort || undefined,
      })
      track(AGENT_CREATED_EVENT, {
        runtime: adapterId,
        model_id: modelId || undefined,
        reasoning_effort: reasoningEffort || undefined,
      })
      setCreateOpen(false)
      setNewName('')
    } catch (err) {
      setCreateError(err instanceof Error ? err.message : String(err))
    }
  }

  const handleDelete = async (item: AgentListItem) => {
    setDeletingAgentKey(item.key)
    setPageError(null)
    try {
      await deleteHarnessAgent.mutateAsync(item.agentId)
      track(AGENT_DELETED_EVENT, {
        runtime: item.source,
        agent_id: item.agentId,
      })
    } catch (err) {
      setPageError(err instanceof Error ? err.message : String(err))
    } finally {
      setDeletingAgentKey(null)
    }
  }

  return (
    <div className="space-y-6">
      <div className="rounded-xl border border-border bg-card p-6 shadow-sm">
        <div className="flex items-start gap-4">
          <div className="flex size-12 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-orange)]/10 text-[var(--accent-orange)]">
            <AdapterIcon adapter={adapterId} className="size-6" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <h2 className="font-semibold text-xl">{label}</h2>
              <AdapterHealthBadge health={adapter?.health} />
            </div>
            <p className="mt-1 text-muted-foreground text-sm">
              {adapter
                ? `Default model ${adapter.defaultModelId} · ${adapter.defaultReasoningEffort} reasoning`
                : 'Runtime details load from the agent server.'}
            </p>
            {adapter?.health ? (
              <AdapterHealthMeta health={adapter.health} />
            ) : null}

            {/* Advanced: Executable Path Configuration */}
            {(adapterId === 'claude' || adapterId === 'codex') && (
              <div className="mt-4 flex flex-col gap-2 rounded-lg border border-border/50 bg-muted/30 p-3">
                <div className="flex items-center gap-2 text-muted-foreground text-xs font-medium uppercase tracking-wider">
                  <Settings2 className="size-3" />
                  Advanced: Executable Path
                </div>
                <div className="flex gap-2">
                  <Input
                    placeholder={`e.g. /usr/local/bin/${adapterId}`}
                    value={executablePath}
                    onChange={(e) => setExecutablePath(e.target.value)}
                    className="h-8 text-xs font-mono"
                  />
                  <Button
                    size="sm"
                    onClick={saveExecutablePath}
                    disabled={isSavingPath}
                    className="h-8 px-3 text-xs"
                  >
                    {isSavingPath ? 'Saving...' : 'Save'}
                  </Button>
                </div>
                <p className="text-[10px] text-muted-foreground italic">
                  Overrides the default PATH-based discovery for this agent.
                </p>
              </div>
            )}
          </div>
          <Button
            onClick={() => setCreateOpen(true)}
            variant="outline"
            className="border-[var(--accent-orange)] bg-[var(--accent-orange)]/10 text-[var(--accent-orange)] hover:bg-[var(--accent-orange)]/20 hover:text-[var(--accent-orange)]"
          >
            <Plus className="mr-1.5 size-4" />
            New Agent
          </Button>
        </div>
      </div>

      {pageError ? (
        <InlineErrorAlert
          message={pageError}
          onDismiss={() => setPageError(null)}
        />
      ) : null}

      <AgentList
        agents={listItems}
        activity={activity}
        harnessAgentLookup={harnessAgentLookup}
        adapters={adapter ? [adapter] : []}
        loading={loading}
        deletingAgentKey={
          deleteHarnessAgent.isPending ? deletingAgentKey : null
        }
        onCreateAgent={() => setCreateOpen(true)}
        onDeleteAgent={(agent) => {
          void handleDelete(agent)
        }}
        onPinToggle={(agent, next) => {
          if (!harnessAgentLookup.has(agent.agentId)) return
          updateHarnessAgent.mutate({
            agentId: agent.agentId,
            patch: { pinned: next },
          })
        }}
      />

      <NewAgentDialog
        adapters={adapter ? [adapter] : []}
        createError={createError}
        createRuntime={adapterId}
        creating={createHarnessAgent.isPending}
        defaultProviderId=""
        harnessAdapterId={adapterId}
        harnessModelId={modelId}
        harnessReasoningEffort={reasoningEffort}
        hermesProviders={[]}
        hermesSelectedProviderId=""
        name={newName}
        open={createOpen}
        onCreate={handleCreate}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) {
            setCreateError(null)
            createHarnessAgent.reset()
          }
        }}
        onRuntimeChange={() => {}}
        onHarnessAdapterChange={() => {}}
        onHarnessModelChange={setModelId}
        onHarnessReasoningChange={setReasoningEffort}
        onHermesProviderChange={() => {}}
        onNameChange={setNewName}
      />
    </div>
  )
}

function AdapterHealthBadge({
  health,
}: {
  health: HarnessAdapterHealth | undefined
}) {
  if (!health) return null
  const tone = adapterHealthTone(health)
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 font-medium text-xs',
        tone === 'ready' &&
          'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400',
        tone === 'warning' &&
          'bg-amber-500/10 text-amber-600 dark:text-amber-400',
        tone === 'danger' && 'bg-red-500/10 text-red-600 dark:text-red-400',
      )}
      title={health.reason}
    >
      <span
        className={cn(
          'size-1.5 rounded-full',
          tone === 'ready' && 'bg-emerald-500',
          tone === 'warning' && 'bg-amber-500',
          tone === 'danger' && 'bg-red-500',
        )}
      />
      {adapterHealthLabel(health)}
    </span>
  )
}

function AdapterHealthMeta({ health }: { health: HarnessAdapterHealth }) {
  const meta = adapterHealthMeta(health)
  if (!meta) return null
  return <p className="mt-1 text-muted-foreground text-xs">{meta}</p>
}
