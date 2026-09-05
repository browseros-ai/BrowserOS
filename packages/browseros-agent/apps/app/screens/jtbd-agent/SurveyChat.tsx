import { Loader2, Send, Square } from 'lucide-react'
import { type FC, type FormEvent, useEffect, useRef, useState } from 'react'
import { MessageResponse } from '@/components/ai-elements/message'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import { cn } from '@/lib/utils'
import type { Message } from './survey-chat.hooks'

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

export const Chat: FC<Props> = ({
  messages,
  isStreaming,
  onSendMessage,
  onStop,
}) => {
  const [input, setInput] = useState('')
  const messagesEndRef = useRef<HTMLDivElement>(null)

  const messagesLength = messages.length
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentionally scroll on message count change
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messagesLength])

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault()
    if (!input.trim() || isStreaming) return
    onSendMessage(input.trim())
    setInput('')
  }

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      handleSubmit(e)
    }
  }

  return (
    <div className="flex h-[calc(100vh-250px)] flex-col rounded-xl border border-border bg-card shadow-sm">
      <div className="flex-1 space-y-4 overflow-y-auto p-4">
        {messages.map((msg) => (
          <MessageBubble key={msg.id} message={msg} />
        ))}
        <div ref={messagesEndRef} />
      </div>

      <form onSubmit={handleSubmit} className="border-border border-t p-4">
        <div className="flex gap-2">
          <Textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Type your response..."
            disabled={isStreaming}
            className="max-h-40 min-h-[44px] resize-none"
            rows={1}
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
            <Button type="submit" disabled={!input.trim()} size="icon">
              <Send className="h-4 w-4" />
            </Button>
          )}
        </div>
      </form>
    </div>
  )
}
