import { ArrowLeft, PanelRight } from 'lucide-react'
import { type FC, useEffect, useMemo, useRef, useState } from 'react'
import { Navigate, useNavigate, useParams, useSearchParams } from 'react-router'
import { Button } from '@/components/ui/button'
import type {
  HarnessAgent,
  HarnessAgentAdapter,
} from '@/entrypoints/app/agents/agent-harness-types'
import type { AgentAdapterHealth } from '@/entrypoints/app/agents/agent-row/agent-row.types'
import {
  cancelHarnessTurn,
  useAgentAdapters,
  useEnqueueHarnessMessage,
  useHarnessAgents,
  useRemoveHarnessQueuedMessage,
  useUpdateHarnessAgent,
} from '@/entrypoints/app/agents/useAgents'
import type { AgentEntry } from '@/entrypoints/app/agents/useOpenClaw'
import { type ProducedFilesRailGroup, useAgentOutputs } from '@/lib/agent-files'
import { cn } from '@/lib/utils'
import { AgentRail } from './AgentRail'
import { useAgentCommandData } from './agent-command-layout'
import {
  OutputsRail,
  useOutputsRailOpen,
} from './agent-conversation.outputs-rail'
import { ClawChat } from './ClawChat'
import { ConversationHeader } from './ConversationHeader'
import { ConversationInput } from './ConversationInput'
import {
  buildChatHistoryFromClawMessages,
  filterTurnsPersistedInHistory,
  flattenHistoryPages,
  mapHistoryToProducedFilesGroups,
  selectStripOnlyTurns,
} from './claw-chat-types'
import { consumePendingInitialMessage } from './pending-initial-message'
import { QueuePanel } from './QueuePanel'
import { useAgentConversation } from './useAgentConversation'
import { useHarnessChatHistory } from './useHarnessChatHistory'

function AgentConversationController({
  agentId,
  initialMessage,
  onInitialMessageConsumed,
  agents,
  agentPathPrefix,
  createAgentPath,
  onOpenOutputsRail,
}: {
  agentId: string
  initialMessage: string | null
  onInitialMessageConsumed: () => void
  agents: AgentEntry[]
  agentPathPrefix: string
  createAgentPath: string
  onOpenOutputsRail?: ((turnId?: string | null) => void) | null
}) {
  const navigate = useNavigate()
  const initialMessageSentRef = useRef<string | null>(null)
  const onInitialMessageConsumedRef = useRef(onInitialMessageConsumed)
  const agent = agents.find((entry) => entry.agentId === agentId)
  const agentName = agent?.name || agentId || 'Agent'
  // Routing is now harness-only. Every OpenClaw agent has a harness
  // record post the gateway → harness backfill, so the chat panel
  // always talks to /agents/<id>/chat. The legacy ClawChat surface
  // was deleted with the /claw/agents/:id/chat server route.
  const harnessHistoryQuery = useHarnessChatHistory(agentId, Boolean(agent))

  const historyMessages = useMemo(
    () =>
      flattenHistoryPages(
        harnessHistoryQuery.data ? [harnessHistoryQuery.data] : [],
      ),
    [harnessHistoryQuery.data],
  )
  const chatHistory = useMemo(
    () => buildChatHistoryFromClawMessages(historyMessages),
    [historyMessages],
  )

  // Listing query feeds queue + active-turn state for this agent. We
  // already poll it every 5s for the rail; reusing the same cache
  // keeps cross-tab queue state in sync without a second poll.
  const { harnessAgents } = useHarnessAgents()
  const harnessAgent = harnessAgents.find((entry) => entry.id === agentId)
  const queue = harnessAgent?.queue ?? []
  const activeTurnId = harnessAgent?.activeTurnId ?? null
  const isOpenClawAgent = harnessAgent?.adapter === 'openclaw'

  // Used to surface produced-files strips on a fresh page load
  // when there's no optimistic turn to carry the data. Disabled
  // for non-openclaw adapters since they don't attribute files.
  const { groups: agentOutputGroups } = useAgentOutputs(
    agentId,
    isOpenClawAgent,
  )

  const { turns, streaming, send } = useAgentConversation(agentId, {
    runtime: 'agent-harness',
    sessionKey: null,
    history: chatHistory,
    activeTurnId,
    onComplete: () => {
      void harnessHistoryQuery.refetch()
    },
    onSessionKeyChange: () => {},
  })
  const enqueueMessage = useEnqueueHarnessMessage()
  const removeQueuedMessage = useRemoveHarnessQueuedMessage()

  const handleStop = () => {
    void cancelHarnessTurn(agentId, {
      turnId: activeTurnId ?? undefined,
      reason: 'user pressed stop',
    })
  }
  const visibleTurns = useMemo(
    () => filterTurnsPersistedInHistory(turns, historyMessages),
    [historyMessages, turns],
  )
  // Persisted turns that still need to surface their FileCardStrip
  // — history items don't carry produced-files data, so without
  // these the strip would vanish on history reload.
  const stripOnlyTurns = useMemo(
    () => selectStripOnlyTurns(turns, historyMessages),
    [historyMessages, turns],
  )
  // Two outputs from the per-turn matcher:
  //  - filesByAssistantId  → strip rendered directly under the
  //    matching assistant history bubble.
  //  - tailUnmatched      → groups with no history pair (orphans);
  //    rendered at the conversation tail.
  // Both are filtered to exclude turnIds already covered by a
  // live or strip-only optimistic turn (those carry their own
  // strip and history hasn't reloaded yet).
  const { filesByAssistantId, tailStripGroups } = useMemo(() => {
    if (!isOpenClawAgent) {
      return {
        filesByAssistantId: new Map<string, ProducedFilesRailGroup>(),
        tailStripGroups: [] as ProducedFilesRailGroup[],
      }
    }
    const coveredTurnIds = new Set<string>()
    for (const turn of turns) {
      if (turn.turnId) coveredTurnIds.add(turn.turnId)
    }
    const eligibleGroups = agentOutputGroups.filter(
      (group) => !coveredTurnIds.has(group.turnId),
    )
    const { byAssistantMessageId, unmatched } = mapHistoryToProducedFilesGroups(
      historyMessages,
      eligibleGroups,
    )
    return {
      filesByAssistantId: byAssistantMessageId,
      tailStripGroups: unmatched,
    }
  }, [agentOutputGroups, isOpenClawAgent, historyMessages, turns])
  onInitialMessageConsumedRef.current = onInitialMessageConsumed

  const disabled = !agent
  const historyReady =
    harnessHistoryQuery.isFetched || harnessHistoryQuery.isError
  const initialMessageKey = initialMessage
    ? `${agentId}:${initialMessage}`
    : null
  const error = harnessHistoryQuery.error ?? null

  const sendRef = useRef(send)
  sendRef.current = send

  useEffect(() => {
    if (disabled || !historyReady) return

    // Registry-first: when the user submitted at /home with
    // attachments, the rich payload is here. URL `?q=` may also be
    // present and is the text-only fallback path; the registry wins
    // when both exist because it carries the binary attachments
    // alongside the text.
    const pending = consumePendingInitialMessage(agentId)
    if (pending) {
      // Mark the dedup ref so the text-only branch below doesn't
      // re-fire on the same render.
      if (initialMessageKey) {
        initialMessageSentRef.current = initialMessageKey
      }
      onInitialMessageConsumedRef.current()
      void sendRef.current({
        text: pending.text,
        attachments: pending.attachments.map((a) => a.payload),
        attachmentPreviews: pending.attachments.map((a) => ({
          id: a.id,
          kind: a.kind,
          mediaType: a.mediaType,
          name: a.name,
          dataUrl: a.dataUrl,
        })),
      })
      return
    }

    const query = initialMessage?.trim()
    if (!initialMessageKey) {
      // Reset is safe even on the post-registry-fire re-run: consume
      // is destructive, so the registry is already drained — there's
      // nothing left for a third run to re-send.
      initialMessageSentRef.current = null
      return
    }

    if (!query || initialMessageSentRef.current === initialMessageKey) {
      return
    }

    initialMessageSentRef.current = initialMessageKey
    onInitialMessageConsumedRef.current()
    void sendRef.current({ text: query })
  }, [agentId, disabled, historyReady, initialMessage, initialMessageKey])

  const handleSelectAgent = (entry: AgentEntry) => {
    navigate(`${agentPathPrefix}/${entry.agentId}`)
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <ClawChat
        agentName={agentName}
        historyMessages={historyMessages}
        turns={visibleTurns}
        stripOnlyTurns={stripOnlyTurns}
        filesByAssistantId={filesByAssistantId}
        tailStripGroups={tailStripGroups}
        streaming={streaming}
        isInitialLoading={harnessHistoryQuery.isLoading}
        error={error}
        hasNextPage={false}
        isFetchingNextPage={false}
        onFetchNextPage={() => {}}
        onOpenOutputsRail={onOpenOutputsRail}
        onRetry={() => {
          void harnessHistoryQuery.refetch()
        }}
      />

      <div className="border-border/50 border-t bg-background/88 px-4 py-3 backdrop-blur-md">
        <div className="mx-auto max-w-3xl space-y-3">
          {queue.length > 0 ? (
            <QueuePanel
              queue={queue}
              onRemove={(messageId) =>
                removeQueuedMessage.mutate({ agentId, messageId })
              }
            />
          ) : null}
          <ConversationInput
            variant="conversation"
            agents={agents}
            selectedAgentId={agentId}
            onSelectAgent={handleSelectAgent}
            onSend={(input) => {
              const attachments = input.attachments.map((a) => a.payload)
              const attachmentPreviews = input.attachments.map((a) => ({
                id: a.id,
                kind: a.kind,
                mediaType: a.mediaType,
                name: a.name,
                dataUrl: a.dataUrl,
              }))
              // When the agent already has an in-flight turn, route
              // the new message into the durable queue instead of
              // starting a parallel turn. Drains automatically as
              // soon as the active turn ends.
              if (streaming || activeTurnId) {
                enqueueMessage.mutate({
                  agentId,
                  message: input.text,
                  attachments,
                })
                return
              }
              void send({ text: input.text, attachments, attachmentPreviews })
            }}
            onCreateAgent={() => navigate(createAgentPath)}
            onStop={handleStop}
            streaming={streaming}
            disabled={disabled}
            status="running"
            attachmentsEnabled={true}
            placeholder={
              streaming
                ? `Type to queue another message for ${agentName}...`
                : `Message ${agentName}...`
            }
          />
        </div>
      </div>
    </div>
  )
}

interface AgentCommandConversationProps {
  variant?: 'command' | 'page'
  backPath?: string
  agentPathPrefix?: string
  createAgentPath?: string
}

function inferAdapterFromEntry(
  entry: AgentEntry | undefined,
): HarnessAgentAdapter | 'unknown' {
  if (!entry) return 'unknown'
  if (entry.source === 'agent-harness') {
    // Harness entries don't carry the adapter on AgentEntry; the rail
    // / header read the harness record directly. This branch only runs
    // before the harness query resolves, so 'unknown' is correct — the
    // tile's bot fallback renders until data arrives.
    return 'unknown'
  }
  // OpenClaw-only entries (no harness shadow) are deprecated in
  // practice but the rail still tolerates them.
  return 'openclaw'
}

export const AgentCommandConversation: FC<AgentCommandConversationProps> = ({
  variant = 'command',
  backPath = '/home',
  agentPathPrefix = '/home/agents',
  createAgentPath = '/agents',
}) => {
  const { agentId } = useParams<{ agentId: string }>()
  const [searchParams, setSearchParams] = useSearchParams()
  const navigate = useNavigate()
  const { agents } = useAgentCommandData()
  const { harnessAgents } = useHarnessAgents()
  const { adapters } = useAgentAdapters()
  const updateAgent = useUpdateHarnessAgent()

  const shouldRedirectHome = !agentId
  const resolvedAgentId = agentId ?? ''
  const harnessAgent = harnessAgents.find(
    (entry) => entry.id === resolvedAgentId,
  )
  const entry = agents.find((item) => item.agentId === resolvedAgentId)
  const fallbackName = entry?.name || resolvedAgentId || 'Agent'
  const fallbackAdapter = inferAdapterFromEntry(entry)
  const initialMessage = searchParams.get('q')
  const isPageVariant = variant === 'page'
  const backLabel = isPageVariant ? 'Back to agents' : 'Back to home'

  const isOpenClawAgent = harnessAgent?.adapter === 'openclaw'
  const [outputsRailOpen, setOutputsRailOpen] =
    useOutputsRailOpen(resolvedAgentId)
  const railVisible = isOpenClawAgent && outputsRailOpen

  // Deep-link target for the rail. Set when (a) the user clicks
  // View / +N on an inline file-card strip, or (b) an external nav
  // arrived with `?outputsTurn=<turnId>`. Cleared by the rail
  // itself once it has scrolled to + expanded the matching group.
  const urlOutputsTurn = searchParams.get('outputsTurn')
  const [focusTurnId, setFocusTurnId] = useState<string | null>(urlOutputsTurn)
  // If the URL param flips while we're already on this agent, sync.
  useEffect(() => {
    if (!urlOutputsTurn) return
    setFocusTurnId(urlOutputsTurn)
    if (isOpenClawAgent) setOutputsRailOpen(true)
  }, [urlOutputsTurn, isOpenClawAgent, setOutputsRailOpen])

  const handleOpenOutputsRail = (turnId?: string | null) => {
    if (!isOpenClawAgent) return
    setOutputsRailOpen(true)
    setFocusTurnId(turnId ?? null)
  }
  const handleFocusTurnConsumed = () => {
    setFocusTurnId(null)
    if (urlOutputsTurn) {
      // Drop the URL param so a back-nav doesn't re-trigger the
      // scroll. `replace: true` keeps history clean.
      setSearchParams(
        (prev) => {
          const next = new URLSearchParams(prev)
          next.delete('outputsTurn')
          return next
        },
        { replace: true },
      )
    }
  }

  const adapterHealth = useMemo<AgentAdapterHealth | null>(() => {
    const adapterId = harnessAgent?.adapter
    if (!adapterId) return null
    const descriptor = adapters.find((item) => item.id === adapterId)
    if (!descriptor?.health) return null
    return {
      healthy: descriptor.health.healthy,
      reason: descriptor.health.reason,
    }
  }, [adapters, harnessAgent?.adapter])

  if (shouldRedirectHome) {
    return <Navigate to="/home" replace />
  }

  const handleSelectHarnessAgent = (target: HarnessAgent) => {
    navigate(`${agentPathPrefix}/${target.id}`)
  }

  const handlePinToggle = (target: HarnessAgent | null, next: boolean) => {
    if (!target) return
    updateAgent.mutate({
      agentId: target.id,
      patch: { pinned: next },
    })
  }

  return (
    <div className="absolute inset-0 overflow-hidden bg-background md:pl-[theme(spacing.14)]">
      <div className="mx-auto flex h-full w-full max-w-[1480px] flex-col">
        {/* Shared top band — the rail's "Agents" header and the chat
            header live on one row so they're aligned by construction. */}
        <div className="flex shrink-0 items-stretch border-border/50 border-b">
          <div className="hidden min-h-[60px] w-[288px] shrink-0 items-center gap-3 border-border/50 border-r px-4 lg:flex">
            <Button
              variant="ghost"
              size="icon"
              onClick={() => navigate(backPath)}
              className="size-8 rounded-xl"
              title="Back to home"
            >
              <ArrowLeft className="size-4" />
            </Button>
            <div className="truncate font-semibold text-[15px] leading-5">
              Agents
            </div>
          </div>
          <div className="min-w-0 flex-1">
            <ConversationHeader
              agent={harnessAgent ?? null}
              fallbackName={fallbackName}
              fallbackAdapter={fallbackAdapter}
              adapterHealth={adapterHealth}
              backLabel={backLabel}
              backTarget={isPageVariant ? 'page' : 'home'}
              onGoHome={() => navigate(backPath)}
              onPinToggle={(next) =>
                handlePinToggle(harnessAgent ?? null, next)
              }
              headerExtra={
                isOpenClawAgent ? (
                  <Button
                    variant={railVisible ? 'secondary' : 'ghost'}
                    size="icon"
                    className="size-8 rounded-xl"
                    onClick={() => setOutputsRailOpen(!railVisible)}
                    title={railVisible ? 'Hide outputs' : 'Show outputs'}
                  >
                    <PanelRight className="size-4" />
                  </Button>
                ) : undefined
              }
            />
          </div>
        </div>

        {/* Body grid: rail list + chat (+ outputs rail when an
            openclaw agent has it open). Columns share the same top
            edge as the band above so headers can never drift. */}
        <div
          className={cn(
            'grid min-h-0 flex-1 grid-rows-[minmax(0,1fr)]',
            railVisible
              ? 'lg:grid-cols-[288px_minmax(0,1fr)_320px]'
              : 'lg:grid-cols-[288px_minmax(0,1fr)]',
          )}
        >
          <AgentRail
            agents={harnessAgents}
            adapters={adapters}
            activeAgentId={resolvedAgentId}
            onSelectAgent={handleSelectHarnessAgent}
            onPinToggle={(target, next) => handlePinToggle(target, next)}
          />

          <div className="flex h-full min-h-0 flex-col overflow-hidden">
            <AgentConversationController
              key={resolvedAgentId}
              agentId={resolvedAgentId}
              agents={agents}
              initialMessage={initialMessage}
              onInitialMessageConsumed={() => {
                // Preserve the outputsTurn deep-link if present —
                // dropping all params would erase the rail focus
                // before it had a chance to consume.
                setSearchParams(
                  (prev) => {
                    const next = new URLSearchParams()
                    const turn = prev.get('outputsTurn')
                    if (turn) next.set('outputsTurn', turn)
                    return next
                  },
                  { replace: true },
                )
              }}
              agentPathPrefix={agentPathPrefix}
              createAgentPath={createAgentPath}
              onOpenOutputsRail={isOpenClawAgent ? handleOpenOutputsRail : null}
            />
          </div>

          {railVisible ? (
            <OutputsRail
              agentId={resolvedAgentId}
              onClose={() => setOutputsRailOpen(false)}
              focusTurnId={focusTurnId}
              onFocusTurnConsumed={handleFocusTurnConsumed}
            />
          ) : null}
        </div>
      </div>
    </div>
  )
}
