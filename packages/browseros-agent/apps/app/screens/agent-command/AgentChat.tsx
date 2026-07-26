import { Bot, Loader2, RefreshCw } from 'lucide-react'
import { type FC, useEffect, useRef } from 'react'
import {
  Conversation,
  ConversationContent,
  ConversationScrollButton,
} from '@/components/ai-elements/conversation'
import type { AgentConversationTurn } from '@/lib/agent-conversations/types'
import { cn } from '@/lib/utils'
import { AgentChatMessage } from './AgentChatMessage'
import type { AgentChatMessage as AgentChatMessageModel } from './agent-chat-types'
import { ConversationMessage } from './ConversationMessage'

export interface AgentChatProps {
  agentName: string
  historyMessages: AgentChatMessageModel[]
  turns: AgentConversationTurn[]
  streaming: boolean
  isInitialLoading: boolean
  error: Error | null
  hasNextPage: boolean
  isFetchingNextPage: boolean
  onFetchNextPage: () => void
  onRetry: () => void
  className?: string
}

function EmptyConversationState({ agentName }: { agentName: string }) {
  return (
    <div className="flex h-full items-center justify-center px-6 py-12">
      <div className="max-w-md text-center">
        <div className="mx-auto flex size-14 items-center justify-center rounded-3xl bg-muted text-muted-foreground">
          <Bot className="size-6" />
        </div>
        <h2 className="mt-5 font-semibold text-xl">{agentName}</h2>
        <p className="mt-2 text-muted-foreground text-sm leading-6">
          Ask {agentName} to start a task.
        </p>
      </div>
    </div>
  )
}

function LoadingConversationState() {
  return (
    <div className="flex h-full items-center justify-center gap-2 text-muted-foreground text-sm">
      <Loader2 className="size-4 animate-spin" />
      Loading conversation...
    </div>
  )
}

function ConversationErrorState({
  message,
  onRetry,
}: {
  message: string
  onRetry: () => void
}) {
  return (
    <div className="flex h-full items-center justify-center px-6 py-12">
      <div className="max-w-md rounded-2xl border border-border/60 bg-card px-5 py-4 text-center shadow-sm">
        <p className="text-sm">{message}</p>
        <button
          type="button"
          onClick={onRetry}
          className="mt-3 inline-flex items-center gap-2 rounded-lg border border-border/60 px-3 py-1.5 font-medium text-muted-foreground text-xs transition-colors hover:bg-accent hover:text-foreground"
        >
          <RefreshCw className="size-3.5" />
          Retry
        </button>
      </div>
    </div>
  )
}

export const AgentChat: FC<AgentChatProps> = ({
  agentName,
  historyMessages,
  turns,
  streaming,
  isInitialLoading,
  error,
  hasNextPage,
  isFetchingNextPage,
  onFetchNextPage,
  onRetry,
  className,
}) => {
  const topSentinelRef = useRef<HTMLDivElement>(null)
  const onFetchNextPageRef = useRef(onFetchNextPage)
  onFetchNextPageRef.current = onFetchNextPage
  const hasMessages = historyMessages.length > 0 || turns.length > 0

  useEffect(() => {
    const sentinel = topSentinelRef.current
    if (!sentinel) return

    const observer = new IntersectionObserver(
      (entries) => {
        const [entry] = entries
        if (!entry?.isIntersecting || !hasNextPage || isFetchingNextPage) {
          return
        }

        onFetchNextPageRef.current()
      },
      {
        root: null,
        rootMargin: '160px 0px 0px 0px',
        threshold: 0,
      },
    )

    observer.observe(sentinel)
    return () => observer.disconnect()
  }, [hasNextPage, isFetchingNextPage])

  return (
    <div
      className={cn('flex min-h-0 flex-1 flex-col overflow-hidden', className)}
    >
      <Conversation
        className={cn(
          'bg-background',
          '[&_[data-streamdown="code-block"]]:!w-full [&_[data-streamdown="code-block"]]:!max-w-full [&_[data-streamdown="table-wrapper"]]:!w-full [&_[data-streamdown="table-wrapper"]]:!max-w-full [&_[data-streamdown="code-block"]]:overflow-x-auto [&_[data-streamdown="table-wrapper"]]:overflow-x-auto',
        )}
      >
        <ConversationContent className="min-h-full px-5 py-5">
          {isInitialLoading ? (
            <LoadingConversationState />
          ) : error && !hasMessages ? (
            <ConversationErrorState message={error.message} onRetry={onRetry} />
          ) : !hasMessages ? (
            <EmptyConversationState agentName={agentName} />
          ) : (
            <div className="mx-auto flex w-full max-w-3xl flex-col gap-3">
              <div ref={topSentinelRef} aria-hidden="true" className="h-px" />
              {isFetchingNextPage ? (
                <div className="flex justify-center py-2 text-muted-foreground text-xs">
                  <Loader2 className="mr-2 size-3.5 animate-spin" />
                  Loading older messages...
                </div>
              ) : null}
              {!hasNextPage && historyMessages.length > 0 ? (
                <div className="py-1 text-center text-muted-foreground text-xs">
                  Start of conversation
                </div>
              ) : null}
              {historyMessages.map((message) => (
                <AgentChatMessage key={message.id} message={message} />
              ))}
              {turns.map((turn, index) => (
                <ConversationMessage
                  key={turn.id}
                  turn={turn}
                  streaming={streaming && index === turns.length - 1}
                />
              ))}
              {error ? (
                <div className="rounded-xl border border-border/60 bg-card px-4 py-3 text-muted-foreground text-sm">
                  {error.message}
                </div>
              ) : null}
            </div>
          )}
        </ConversationContent>
        <ConversationScrollButton />
      </Conversation>
    </div>
  )
}
