import { ArrowLeft, Bot, Home } from 'lucide-react'
import { type FC, useEffect, useMemo, useRef } from 'react'
import { Navigate, useNavigate, useParams, useSearchParams } from 'react-router'
import { Button } from '@/components/ui/button'
import {
  type AgentEntry,
  getModelDisplayName,
} from '@/entrypoints/app/agents/useOpenClaw'
import { cn } from '@/lib/utils'
import { useAgentCommandData } from './agent-command-layout'
import { ClawChat } from './ClawChat'
import { ConversationInput } from './ConversationInput'
import {
  buildChatHistoryFromClawMessages,
  filterTurnsPersistedInHistory,
  flattenHistoryPages,
} from './claw-chat-types'
import { useAgentConversation } from './useAgentConversation'
import { useHarnessChatHistory } from './useHarnessChatHistory'

function StatusBadge({ status }: { status: string }) {
  return (
    <div className="inline-flex items-center gap-2 rounded-full border border-border/60 bg-card px-3 py-1 text-[11px] text-muted-foreground uppercase tracking-[0.18em]">
      <span
        className={cn(
          'size-1.5 rounded-full',
          status === 'Working on your request'
            ? 'bg-amber-500'
            : status === 'Ready'
              ? 'bg-emerald-500'
              : status === 'Offline'
                ? 'bg-muted-foreground/50'
                : 'bg-[var(--accent-orange)]',
        )}
      />
      <span>{status}</span>
    </div>
  )
}

function AgentIdentity({
  name,
  meta,
  className,
}: {
  name: string
  meta: string
  className?: string
}) {
  return (
    <div className={cn('min-w-0', className)}>
      <div className="truncate font-semibold text-[15px] leading-5">{name}</div>
      <div className="truncate text-muted-foreground text-xs leading-5">
        {meta}
      </div>
    </div>
  )
}

function ConversationHeader({
  agentName,
  agentMeta,
  status,
  backLabel,
  backTarget,
  onGoHome,
}: {
  agentName: string
  agentMeta: string
  status: string
  backLabel: string
  backTarget: 'home' | 'page'
  onGoHome: () => void
}) {
  const BackIcon = backTarget === 'home' ? Home : ArrowLeft

  return (
    <div className="flex h-14 items-center justify-between gap-4 border-border/50 border-b px-5">
      <div className="flex min-w-0 items-center gap-3">
        <Button
          variant="ghost"
          size="icon"
          onClick={onGoHome}
          className="size-8 rounded-xl lg:hidden"
          title={backLabel}
        >
          <BackIcon className="size-4" />
        </Button>
        <div className="flex size-8 shrink-0 items-center justify-center rounded-xl bg-muted text-muted-foreground">
          <Bot className="size-4" />
        </div>
        <AgentIdentity name={agentName} meta={agentMeta} />
      </div>

      <StatusBadge status={status} />
    </div>
  )
}

function AgentRailHeader({ onGoHome }: { onGoHome: () => void }) {
  return (
    <div className="hidden h-14 items-center border-border/50 border-r border-b bg-background/70 px-4 lg:flex">
      <div className="flex min-w-0 items-center gap-3">
        <Button
          variant="ghost"
          size="icon"
          onClick={onGoHome}
          className="size-8 rounded-xl"
          title="Back to home"
        >
          <ArrowLeft className="size-4" />
        </Button>
        <div className="truncate font-semibold text-[15px] leading-5">
          Agents
        </div>
      </div>
    </div>
  )
}

function AgentRailList({
  activeAgentId,
  agents,
  onSelectAgent,
}: {
  activeAgentId: string
  agents: AgentEntry[]
  onSelectAgent: (entry: AgentEntry) => void
}) {
  return (
    <aside className="hidden min-h-0 flex-col border-border/50 border-r bg-background/70 lg:flex">
      <div className="styled-scrollbar min-h-0 flex-1 space-y-2 overflow-y-auto px-3 py-3">
        {agents.map((entry) => {
          const active = entry.agentId === activeAgentId
          const modelName = getAgentEntryMeta(entry)

          return (
            <button
              key={entry.agentId}
              type="button"
              onClick={() => onSelectAgent(entry)}
              className={cn(
                'w-full rounded-2xl border px-3 py-3 text-left transition-all',
                active
                  ? 'border-[var(--accent-orange)]/30 bg-[var(--accent-orange)]/8 shadow-sm'
                  : 'border-transparent bg-transparent hover:border-border/60 hover:bg-card',
              )}
            >
              <div className="flex items-center gap-3">
                <div
                  className={cn(
                    'flex size-9 items-center justify-center rounded-xl',
                    active
                      ? 'bg-[var(--accent-orange)]/12 text-[var(--accent-orange)]'
                      : 'bg-muted text-muted-foreground',
                  )}
                >
                  <Bot className="size-4" />
                </div>
                <AgentIdentity name={entry.name} meta={modelName} />
              </div>
            </button>
          )
        })}
      </div>
    </aside>
  )
}

function getAgentEntryMeta(agent: AgentEntry | undefined): string {
  if (agent?.source === 'agent-harness') {
    return getModelDisplayName(agent.model) ?? 'ACP agent'
  }
  return getModelDisplayName(agent?.model) ?? 'OpenClaw agent'
}

function AgentConversationController({
  agentId,
  initialMessage,
  onInitialMessageConsumed,
  agents,
  agentPathPrefix,
  createAgentPath,
}: {
  agentId: string
  initialMessage: string | null
  onInitialMessageConsumed: () => void
  agents: AgentEntry[]
  agentPathPrefix: string
  createAgentPath: string
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

  const { turns, streaming, send } = useAgentConversation(agentId, {
    runtime: 'agent-harness',
    sessionKey: null,
    history: chatHistory,
    onComplete: () => {
      void harnessHistoryQuery.refetch()
    },
    onSessionKeyChange: () => {},
  })
  const visibleTurns = useMemo(
    () => filterTurnsPersistedInHistory(turns, historyMessages),
    [historyMessages, turns],
  )
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
    const query = initialMessage?.trim()
    if (!initialMessageKey) {
      initialMessageSentRef.current = null
      return
    }

    if (
      !query ||
      initialMessageSentRef.current === initialMessageKey ||
      disabled ||
      !historyReady
    ) {
      return
    }

    initialMessageSentRef.current = initialMessageKey
    onInitialMessageConsumedRef.current()
    void sendRef.current({ text: query })
  }, [disabled, historyReady, initialMessage, initialMessageKey])

  const handleSelectAgent = (entry: AgentEntry) => {
    navigate(`${agentPathPrefix}/${entry.agentId}`)
  }

  return (
    <div className="flex min-h-0 flex-col overflow-hidden">
      <ClawChat
        agentName={agentName}
        historyMessages={historyMessages}
        turns={visibleTurns}
        streaming={streaming}
        isInitialLoading={harnessHistoryQuery.isLoading}
        error={error}
        hasNextPage={false}
        isFetchingNextPage={false}
        onFetchNextPage={() => {}}
        onRetry={() => {
          void harnessHistoryQuery.refetch()
        }}
      />

      <div className="border-border/50 border-t bg-background/88 px-4 py-3 backdrop-blur-md">
        <div className="mx-auto max-w-3xl">
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
              void send({ text: input.text, attachments, attachmentPreviews })
            }}
            onCreateAgent={() => navigate(createAgentPath)}
            streaming={streaming}
            disabled={disabled}
            status="running"
            attachmentsEnabled={true}
            placeholder={`Message ${agentName}...`}
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
  const shouldRedirectHome = !agentId
  const resolvedAgentId = agentId ?? ''
  const agent = agents.find((entry) => entry.agentId === resolvedAgentId)
  const agentName = agent?.name || resolvedAgentId || 'Agent'
  const agentMeta = getAgentEntryMeta(agent)
  const initialMessage = searchParams.get('q')
  const isPageVariant = variant === 'page'
  const backLabel = isPageVariant ? 'Back to agents' : 'Back to home'

  if (shouldRedirectHome) {
    return <Navigate to="/home" replace />
  }

  const handleSelectAgent = (entry: AgentEntry) => {
    navigate(`${agentPathPrefix}/${entry.agentId}`)
  }

  // Every visible agent runs through the harness now, so per-agent
  // runtime status doesn't gate chat the way OpenClaw's legacy
  // gateway lifecycle did. Show "Ready" once the agent record is
  // resolved from the rail, "Setup" otherwise.
  const statusCopy = agent ? 'Ready' : 'Setup'

  return (
    <div className="absolute inset-0 overflow-hidden bg-background md:pl-[theme(spacing.14)]">
      <div className="mx-auto grid h-full w-full max-w-[1480px] lg:grid-cols-[288px_minmax(0,1fr)] lg:grid-rows-[3.5rem_minmax(0,1fr)]">
        <AgentRailHeader onGoHome={() => navigate(backPath)} />

        <ConversationHeader
          agentName={agentName}
          agentMeta={agentMeta}
          status={statusCopy}
          backLabel={backLabel}
          backTarget={isPageVariant ? 'page' : 'home'}
          onGoHome={() => navigate(backPath)}
        />

        <AgentRailList
          activeAgentId={resolvedAgentId}
          agents={agents}
          onSelectAgent={handleSelectAgent}
        />

        <AgentConversationController
          key={resolvedAgentId}
          agentId={resolvedAgentId}
          agents={agents}
          initialMessage={initialMessage}
          onInitialMessageConsumed={() =>
            setSearchParams({}, { replace: true })
          }
          agentPathPrefix={agentPathPrefix}
          createAgentPath={createAgentPath}
        />
      </div>
    </div>
  )
}
