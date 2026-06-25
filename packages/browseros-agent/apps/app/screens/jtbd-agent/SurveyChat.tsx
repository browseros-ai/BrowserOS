import { Loader2, Send, Square } from 'lucide-react'
import { type FC, type FormEvent, useEffect, useRef, useState } from 'react'
import { MessageResponse } from '@/components/ai-elements/message'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import { cn } from '@/lib/utils'
import { useVoiceInput } from '@/modules/voice/voice.hooks'
import type { Message } from './survey-chat.hooks'
import { VoiceInputButton } from './VoiceInputButton'

export interface Props {
  messages: Message[]
  isStreaming: boolean
  onSendMessage: (text: string) => void
  onStop: () => void
}

const MessageBubble: FC<{ message: Message }> = ({ message }) => {
  const isUser = message.role === 'user'

  return (
    <div className={cn('flex', isUser ? 'justify-end' : 'justify-start')}>
      <div
        className={cn(
          'max-w-[80%] rounded-2xl px-4 py-3 text-sm',
          isUser
            ? 'bg-primary text-primary-foreground'
            : 'bg-muted/50 text-foreground',
        )}
      >
        {message.content ? (
          <MessageResponse>{message.content}</MessageResponse>
        ) : (
          <span className="flex items-center gap-2 text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Thinking...
          </span>
        )}
      </div>
    </div>
  )
}

const WAVEFORM_BARS = [0, 1, 2, 3, 4] as const

const WaveformIndicator: FC<{ level: number }> = ({ level }) => {
  return (
    <div className="flex items-center justify-center gap-1">
      {WAVEFORM_BARS.map((barIndex) => {
        const barLevel = Math.max(
          0.2,
          Math.sin((barIndex / WAVEFORM_BARS.length) * Math.PI) * (level / 100),
        )
        return (
          <div
            key={barIndex}
            className="w-1 rounded-full bg-destructive transition-all duration-75"
            style={{ height: `${Math.max(4, barLevel * 20)}px` }}
          />
        )
      })}
    </div>
  )
}

export const Chat: FC<Props> = ({
  messages,
  isStreaming,
  onSendMessage,
  onStop,
}) => {
  const [input, setInput] = useState('')
  const messagesEndRef = useRef<HTMLDivElement>(null)

  const voice = useVoiceInput()

  const messagesLength = messages.length
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentionally scroll on message count change
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messagesLength])

  // Insert transcript into input when transcription completes
  // biome-ignore lint/correctness/useExhaustiveDependencies: only trigger on transcript/transcribing change
  useEffect(() => {
    if (voice.transcript && !voice.isTranscribing) {
      setInput((prev) => {
        const separator = prev.trim() ? ' ' : ''
        return prev + separator + voice.transcript
      })
      voice.clearTranscript()
    }
  }, [voice.transcript, voice.isTranscribing])

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault()
    if (!input.trim() || isStreaming || voice.isRecording) return
    onSendMessage(input.trim())
    setInput('')
  }

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      handleSubmit(e)
    }
  }

  const isInputDisabled =
    isStreaming || voice.isRecording || voice.isTranscribing

  return (
    <div className="flex h-[calc(100vh-250px)] flex-col rounded-xl border border-border bg-card shadow-sm">
      <div className="flex-1 space-y-4 overflow-y-auto p-4">
        {messages.map((msg) => (
          <MessageBubble key={msg.id} message={msg} />
        ))}
        <div ref={messagesEndRef} />
      </div>

      <form onSubmit={handleSubmit} className="border-border border-t p-4">
        {voice.error && (
          <div className="mb-2 text-destructive text-sm">{voice.error}</div>
        )}

        <div className="flex gap-2">
          {voice.isRecording ? (
            <div className="flex flex-1 items-center justify-center gap-2 rounded-md border border-input bg-muted/50 px-3 py-2">
              <WaveformIndicator level={voice.audioLevel} />
              <span className="text-muted-foreground text-sm">
                Listening...
              </span>
            </div>
          ) : (
            <Textarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder={
                voice.isTranscribing
                  ? 'Transcribing...'
                  : 'Type your response...'
              }
              disabled={isInputDisabled}
              className="max-h-40 min-h-[44px] resize-none"
              rows={1}
            />
          )}

          <VoiceInputButton
            isRecording={voice.isRecording}
            isTranscribing={voice.isTranscribing}
            audioLevel={voice.audioLevel}
            disabled={isStreaming}
            onStart={voice.startRecording}
            onStop={voice.stopRecording}
          />

          {isStreaming ? (
            <Button
              type="button"
              onClick={onStop}
              variant="destructive"
              size="icon"
            >
              <Square className="h-4 w-4" />
            </Button>
          ) : (
            <Button
              type="submit"
              disabled={
                !input.trim() || voice.isRecording || voice.isTranscribing
              }
              size="icon"
            >
              <Send className="h-4 w-4" />
            </Button>
          )}
        </div>
      </form>
    </div>
  )
}
