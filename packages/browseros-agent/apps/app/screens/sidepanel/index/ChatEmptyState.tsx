import { ArrowRight, Pencil, Sparkles } from 'lucide-react'
import { type FC, useEffect, useRef, useState } from 'react'
import { sentry } from '@/lib/sentry/sentry'
import { cn } from '@/lib/utils'
import type { ChatMode } from '@/modules/chat/chat-types'
import {
  STARTER_PROMPT_SLOTS,
  type StarterPrompt,
} from '@/modules/chat/starter-prompts'
import { useStarterPrompts } from '@/modules/chat/starter-prompts.hooks'
import { saveStarterPrompts } from '@/modules/chat/starter-prompts-storage'
import { StarterPromptEditor } from './StarterPromptEditor'

export interface ChatEmptyStateProps {
  mode: ChatMode
  mounted: boolean
  onSuggestionClick: (suggestion: string) => void
}

/** Shared by side-panel and new-tab chat; mode changes discard unsaved drafts. */
export const ChatEmptyState: FC<ChatEmptyStateProps> = (props) => (
  <ModeEmptyState key={props.mode} {...props} />
)

const ModeEmptyState: FC<ChatEmptyStateProps> = ({
  mode,
  mounted,
  onSuggestionClick,
}) => {
  const { prompts, isLoading, loadError, retry } = useStarterPrompts(mode)
  const [editing, setEditing] = useState(false)
  const customizeRef = useRef<HTMLButtonElement>(null)
  const restoreFocus = useRef(false)

  useEffect(() => {
    if (!editing && restoreFocus.current) {
      customizeRef.current?.focus()
      restoreFocus.current = false
    }
  }, [editing])

  const finishEditing = () => {
    restoreFocus.current = true
    setEditing(false)
  }

  const save = async (draft: StarterPrompt[]) => {
    try {
      await saveStarterPrompts(mode, draft)
      finishEditing()
    } catch (error) {
      sentry.captureException(error, {
        extra: { message: 'Failed to save starter prompts', mode },
      })
      throw error
    }
  }

  // Grow with the editor in short panels; shrinking this centered flex child
  // would place the form's top above the parent's reachable scroll area.
  return (
    <div
      className={cn(
        'm-0! flex min-h-full w-full shrink-0 flex-col items-center justify-center gap-4 py-4 text-center opacity-0 transition-all duration-700',
        mounted ? 'translate-y-0 opacity-100' : 'translate-y-4 opacity-0',
      )}
    >
      {editing ? (
        <StarterPromptEditor
          prompts={prompts}
          onSave={save}
          onCancel={finishEditing}
        />
      ) : (
        <>
          <div className="mb-2 flex h-14 w-14 items-center justify-center rounded-2xl bg-muted/50">
            <Sparkles className="h-7 w-7 text-[var(--accent-orange)]" />
          </div>
          <div>
            <h2 className="mb-1 font-semibold text-lg">
              {mode === 'chat'
                ? 'Chat with this page'
                : 'Agent at your service'}
            </h2>
            <p className="max-w-[230px] text-muted-foreground text-xs">
              {mode === 'chat'
                ? 'Ask questions about the current page or any topic'
                : 'Let AI automate tasks and browse for you'}
            </p>
          </div>
          <div className="group/prompts mt-6 flex w-full max-w-[320px] flex-col gap-2">
            {STARTER_PROMPT_SLOTS.map((slot) => (
              <button
                type="button"
                key={slot}
                onClick={() => onSuggestionClick(prompts[slot].prompt)}
                className="group flex min-h-11 items-center justify-between gap-3 rounded-lg border border-border/50 bg-card px-3 py-2.5 text-left text-[13px] transition-colors hover:border-[var(--accent-orange)]/50 hover:bg-[var(--accent-orange)]/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <span className="min-w-0 flex-1 break-words">
                  {prompts[slot].display}
                </span>
                <ArrowRight
                  aria-hidden="true"
                  className="size-3.5 shrink-0 text-muted-foreground opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100"
                />
              </button>
            ))}
            {/* Opacity reserves the hover target's space and keeps it keyboard
                reachable. Touch users get the same action without needing hover. */}
            <button
              ref={customizeRef}
              type="button"
              disabled={isLoading || loadError}
              onClick={() => setEditing(true)}
              className="flex items-center justify-center gap-1.5 rounded py-2 text-muted-foreground text-xs opacity-0 transition-opacity hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-wait group-focus-within/prompts:opacity-100 group-hover/prompts:opacity-100 [@media(hover:none)]:opacity-100"
            >
              <Pencil aria-hidden="true" className="size-3" />
              Customize prompts
            </button>
            {loadError && (
              <p role="alert" className="text-destructive text-xs">
                Could not load saved prompts.{' '}
                <button type="button" onClick={retry} className="underline">
                  Retry
                </button>
              </p>
            )}
          </div>
        </>
      )}
    </div>
  )
}
