import { History, Sparkles } from 'lucide-react'
import { type FC, useState } from 'react'
import { cn } from '@/lib/utils'
import {
  AGENT_SUGGESTIONS,
  CHAT_SUGGESTIONS,
  type ChatMode,
} from '@/modules/chat/chat-types'

export interface ChatEmptyStateProps {
  mode: ChatMode
  mounted: boolean
  onSuggestionClick: (suggestion: string) => void
  hasRestorableSession?: boolean
  onRestoreSession?: () => Promise<void>
}

export const ChatEmptyState: FC<ChatEmptyStateProps> = ({
  mode,
  mounted,
  onSuggestionClick,
  hasRestorableSession = false,
  onRestoreSession,
}) => {
  const suggestions = mode === 'chat' ? CHAT_SUGGESTIONS : AGENT_SUGGESTIONS
  const [isRestoring, setIsRestoring] = useState(false)

  const handleRestore = async () => {
    if (!onRestoreSession || isRestoring) return
    setIsRestoring(true)
    await onRestoreSession()
    setIsRestoring(false)
  }

  return (
    <div
      className={cn(
        'm-0! flex h-full flex-col items-center justify-center space-y-4 text-center opacity-0 transition-all duration-700',
        mounted ? 'translate-y-0 opacity-100' : 'translate-y-4 opacity-0',
      )}
    >
      <div className="mb-2 flex h-14 w-14 items-center justify-center rounded-2xl bg-muted/50">
        <Sparkles className="h-7 w-7 text-[var(--accent-orange)]" />
      </div>
      <div>
        <h2 className="mb-1 font-semibold text-lg">
          {mode === 'chat' ? 'Chat with this page' : 'Agent at your service'}
        </h2>
        <p className="max-w-[200px] text-muted-foreground text-xs">
          {mode === 'chat'
            ? 'Ask questions about the current page or any topic'
            : 'Let AI automate tasks and browse for you'}
        </p>
      </div>

      {/* Restore pill — only shown when another tab has an active conversation */}
      {hasRestorableSession && onRestoreSession && (
        <button
          type="button"
          id="restore-chats-pill"
          onClick={handleRestore}
          disabled={isRestoring}
          className={cn(
            'inline-flex items-center gap-1.5 rounded-full border border-border/60 bg-muted/60 px-3.5 py-1.5 text-xs font-medium text-foreground/80 shadow-sm backdrop-blur-sm',
            'transition-all duration-200',
            'hover:border-[var(--accent-orange)]/60 hover:bg-[var(--accent-orange)]/10 hover:text-foreground hover:shadow-md',
            'active:scale-95',
            isRestoring && 'cursor-wait opacity-60',
          )}
        >
          <History
            className={cn(
              'h-3.5 w-3.5 shrink-0',
              isRestoring && 'animate-spin',
            )}
          />
          {isRestoring ? 'Restoring…' : 'Restore chats from other tabs'}
        </button>
      )}

      <div className="mt-6 grid w-full max-w-[260px] grid-cols-1 gap-2">
        {suggestions.map((suggestion) => (
          <button
            type="button"
            key={suggestion.display}
            onClick={() => onSuggestionClick(suggestion.prompt)}
            className="group flex items-center justify-between rounded-lg border border-border/50 bg-card px-3 py-2.5 text-left text-xs transition-all duration-200 hover:border-[var(--accent-orange)]/50 hover:bg-[var(--accent-orange)]/5"
          >
            {suggestion.display}
            <span className="opacity-0 transition-opacity duration-200 group-hover:opacity-100">
              {suggestion.icon}
            </span>
          </button>
        ))}
      </div>
    </div>
  )
}
