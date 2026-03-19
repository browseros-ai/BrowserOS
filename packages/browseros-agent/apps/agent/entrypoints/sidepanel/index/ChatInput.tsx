import { Loader2, Mic, Send, Square, SquareStop } from 'lucide-react'
import type { FormEvent, KeyboardEvent } from 'react'
import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from 'react'
import { TabPickerPopover } from '@/components/elements/tab-picker-popover'
import { cn } from '@/lib/utils'
import type { VoiceInputState } from '@/lib/voice/useVoiceInput'
import type { ChatMode } from './chatTypes'

interface MentionState {
  isOpen: boolean
  filterText: string
  startPosition: number
}

interface ChatInputProps {
  input: string
  status: 'streaming' | 'submitted' | 'ready' | 'error'
  mode: ChatMode
  onInputChange: (value: string) => void
  onSubmit: (e: FormEvent) => void
  onStop: () => void
  selectedTabs: chrome.tabs.Tab[]
  onToggleTab: (tab: chrome.tabs.Tab) => void
  onTabMentionOpenChange?: (isOpen: boolean) => void
  voice?: VoiceInputState
}

export interface ChatInputHandle {
  openTabMention: () => void
  closeTabMention: () => void
  toggleTabMention: () => void
  focus: () => void
}

export const ChatInput = forwardRef<ChatInputHandle, ChatInputProps>(
  (
    {
      input,
      status,
      mode,
      onInputChange,
      onSubmit: onSubmitProp,
      onStop,
      selectedTabs,
      onToggleTab,
      onTabMentionOpenChange,
      voice,
    },
    ref,
  ) => {
    const textareaRef = useRef<HTMLTextAreaElement>(null)
    const [mentionState, setMentionState] = useState<MentionState>({
      isOpen: false,
      filterText: '',
      startPosition: 0,
    })

    const inputRef = useRef(input)
    const mentionStateRef = useRef(mentionState)

    useEffect(() => {
      inputRef.current = input
      mentionStateRef.current = mentionState
    })

    useEffect(() => {
      onTabMentionOpenChange?.(mentionState.isOpen)
    }, [mentionState.isOpen, onTabMentionOpenChange])

    const closeMention = useCallback(() => {
      const state = mentionStateRef.current
      if (state.isOpen) {
        const currentInput = inputRef.current
        const beforeMention = currentInput.slice(0, state.startPosition)
        const afterMention = currentInput.slice(
          state.startPosition + 1 + state.filterText.length,
        )
        const nextInput = beforeMention + afterMention
        inputRef.current = nextInput
        onInputChange(nextInput)
        const nextMentionState = {
          isOpen: false,
          filterText: '',
          startPosition: 0,
        }
        mentionStateRef.current = nextMentionState
        setMentionState(nextMentionState)

        requestAnimationFrame(() => {
          textareaRef.current?.focus()
          const newPosition = beforeMention.length
          textareaRef.current?.setSelectionRange(newPosition, newPosition)
        })
      }
    }, [onInputChange])

    const openMentionAtCursor = useCallback(() => {
      const textarea = textareaRef.current
      if (!textarea) return

      textarea.focus()
      if (mentionStateRef.current.isOpen) return

      const currentInput = inputRef.current
      const cursorPosition = textarea.selectionStart ?? currentInput.length
      const beforeCursor = currentInput.slice(0, cursorPosition)
      const afterCursor = currentInput.slice(cursorPosition)

      const nextInput = `${beforeCursor}@${afterCursor}`
      inputRef.current = nextInput
      onInputChange(nextInput)
      const nextMentionState = {
        isOpen: true,
        filterText: '',
        startPosition: cursorPosition,
      }
      mentionStateRef.current = nextMentionState
      setMentionState(nextMentionState)

      requestAnimationFrame(() => {
        textarea.focus()
        const newPosition = cursorPosition + 1
        textarea.setSelectionRange(newPosition, newPosition)
      })
    }, [onInputChange])

    const toggleMentionAtCursor = useCallback(() => {
      if (mentionStateRef.current.isOpen) {
        closeMention()
        return
      }
      openMentionAtCursor()
    }, [closeMention, openMentionAtCursor])

    useImperativeHandle(
      ref,
      () => ({
        openTabMention: openMentionAtCursor,
        closeTabMention: closeMention,
        toggleTabMention: toggleMentionAtCursor,
        focus: () => textareaRef.current?.focus(),
      }),
      [closeMention, openMentionAtCursor, toggleMentionAtCursor],
    )

    const isBusy = status !== 'ready' && status !== 'error'

    const handleSubmit = (e: FormEvent) => {
      if (mentionStateRef.current.isOpen) {
        e.preventDefault()
        closeMention()
        return
      }
      if (isBusy) {
        e.preventDefault()
        return
      }
      onSubmitProp(e)
    }

    const handleInputChange = (value: string) => {
      const textarea = textareaRef.current
      const cursorPosition = textarea?.selectionStart ?? value.length

      const state = mentionStateRef.current

      if (state.isOpen) {
        const textAfterAt = value.slice(state.startPosition + 1)
        const spaceIndex = textAfterAt.search(/\s/)
        const filterText =
          spaceIndex === -1 ? textAfterAt : textAfterAt.slice(0, spaceIndex)

        if (
          cursorPosition <= state.startPosition ||
          value[state.startPosition] !== '@'
        ) {
          const nextMentionState = {
            isOpen: false,
            filterText: '',
            startPosition: 0,
          }
          mentionStateRef.current = nextMentionState
          setMentionState(nextMentionState)
        } else {
          const nextMentionState = { ...state, filterText }
          mentionStateRef.current = nextMentionState
          setMentionState(nextMentionState)
        }
      } else {
        const charBeforeCursor = value[cursorPosition - 1]
        const textBeforeAt = value.slice(0, cursorPosition - 1)
        const isAtWordBoundary = /(?:^|[\s\n])$/.test(textBeforeAt)

        if (charBeforeCursor === '@' && isAtWordBoundary) {
          const nextMentionState = {
            isOpen: true,
            filterText: '',
            startPosition: cursorPosition - 1,
          }
          mentionStateRef.current = nextMentionState
          setMentionState(nextMentionState)
        }
      }

      inputRef.current = value
      onInputChange(value)
    }

    const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if (mentionState.isOpen) {
        if (
          e.key === 'ArrowDown' ||
          e.key === 'ArrowUp' ||
          e.key === 'Enter' ||
          e.key === 'Escape'
        ) {
          return
        }
        if (e.key === 'Tab') {
          e.preventDefault()
          closeMention()
          return
        }
      }

      if (
        e.key === 'Enter' &&
        !e.shiftKey &&
        !e.metaKey &&
        !e.ctrlKey &&
        !e.nativeEvent.isComposing
      ) {
        e.preventDefault()
        if (input.trim() && !isBusy) {
          e.currentTarget.form?.requestSubmit()
        }
      }
    }

    useEffect(() => {
      if (!mentionState.isOpen) return

      const handleClickOutside = (e: MouseEvent) => {
        const target = e.target as HTMLElement
        if (target.closest('[data-tab-mention-trigger]')) return
        if (
          !textareaRef.current?.contains(target) &&
          !target.closest('[data-slot="popover-content"]')
        ) {
          closeMention()
        }
      }

      document.addEventListener('mousedown', handleClickOutside)
      return () => document.removeEventListener('mousedown', handleClickOutside)
    }, [mentionState.isOpen, closeMention])

    const renderVoiceButton = () => {
      if (!voice) return null

      if (voice.isRecording) {
        return (
          <button
            type="button"
            onClick={voice.onStopRecording}
            className="cursor-pointer rounded-full bg-red-600 p-2 text-white shadow-sm transition-all duration-200 hover:bg-red-900"
          >
            <Square className="h-3.5 w-3.5" />
            <span className="sr-only">Stop recording</span>
          </button>
        )
      }

      if (voice.isTranscribing) {
        return (
          <button
            type="button"
            disabled
            className="rounded-full p-2 text-muted-foreground"
          >
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            <span className="sr-only">Transcribing</span>
          </button>
        )
      }

      return (
        <button
          type="button"
          onClick={voice.onStartRecording}
          disabled={isBusy}
          className="cursor-pointer rounded-full p-2 text-muted-foreground transition-all duration-200 hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
        >
          <Mic className="h-3.5 w-3.5" />
          <span className="sr-only">Voice input</span>
        </button>
      )
    }

    const renderSendButton = () => {
      if (isBusy) {
        return (
          <button
            type="button"
            onClick={onStop}
            className="cursor-pointer rounded-full bg-red-600 p-2 text-white shadow-sm transition-all duration-200 hover:bg-red-900"
          >
            <SquareStop className="h-3.5 w-3.5" />
            <span className="sr-only">Stop</span>
          </button>
        )
      }

      return (
        <button
          type="submit"
          disabled={
            !input.trim() || voice?.isRecording || voice?.isTranscribing
          }
          className="cursor-pointer rounded-full bg-[var(--accent-orange)] p-2 text-white shadow-sm transition-all duration-200 hover:bg-[var(--accent-orange-bright)] disabled:cursor-not-allowed disabled:opacity-50"
        >
          <Send className="h-3.5 w-3.5" />
          <span className="sr-only">Send</span>
        </button>
      )
    }

    return (
      <form
        onSubmit={handleSubmit}
        className="relative mt-2 flex w-full items-end gap-2"
      >
        <TabPickerPopover
          variant="mention"
          isOpen={mentionState.isOpen}
          filterText={mentionState.filterText}
          selectedTabs={selectedTabs}
          onToggleTab={onToggleTab}
          onClose={closeMention}
          anchorRef={textareaRef}
        />
        {voice?.isRecording ? (
          <div className="flex min-h-[42px] flex-1 items-center justify-center gap-1 rounded-2xl border border-red-500/50 bg-muted/50 px-4 py-2.5 pr-[4.5rem]">
            {voice.audioLevels.map((level, i) => (
              <div
                key={i.toString()}
                className="w-1 rounded-full bg-red-500 transition-all duration-75"
                style={{
                  height: `${Math.max(4, Math.min(20, level * 0.6))}px`,
                }}
              />
            ))}
          </div>
        ) : (
          <textarea
            ref={textareaRef}
            className={cn(
              'field-sizing-content max-h-60 min-h-[42px] flex-1 resize-none overflow-hidden rounded-2xl border border-border/50 bg-muted/50 px-4 py-2.5 text-sm outline-none transition-colors placeholder:text-muted-foreground/70 hover:border-border focus:border-[var(--accent-orange)]',
              voice ? 'pr-[4.5rem]' : 'pr-11',
            )}
            value={input}
            onChange={(e) => handleInputChange(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={
              voice?.isTranscribing
                ? 'Transcribing...'
                : mode === 'chat'
                  ? 'Ask about this page...'
                  : 'What should I do?'
            }
            disabled={voice?.isTranscribing}
            rows={1}
          />
        )}
        <div className="absolute right-1.5 bottom-1.5 flex items-center gap-1">
          {renderVoiceButton()}
          {renderSendButton()}
        </div>
      </form>
    )
  },
)

ChatInput.displayName = 'ChatInput'
